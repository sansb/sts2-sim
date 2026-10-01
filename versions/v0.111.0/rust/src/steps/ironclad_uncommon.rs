//! Card-step bodies for the `content/cards/ironclad_uncommon.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status: 14 of 15 ported, 1 still escalated
//!
//! Batch 1 (#1326) escalated all fifteen. Engine slice 2 (#1367) landed the
//! primitives seven of them were waiting on — the step-callable Draw,
//! `_discard_and_draw`, the exhaust pile op, the multi-target attack command,
//! X-cost energy resolution, the 0-cost-attack counter and the powers
//! framework — and the arg-shape gate that blocked the whole wave went
//! body-owned in #1366. Those seven bodies are ported below. Engine slice 3's
//! first seam adds `exhaust_draw` and the second-card action dimension.
//!
//! Batch 1b re-derived the other seven against **that** tree rather than
//! against the state of the engine when they were escalated. At the time none
//! closed. Later Unit C work supplied the typed ally roster, live CardPlay
//! target, and exact local/remote command routes; issue #1561 R29 uses that
//! foundation to close Blaze. The remaining stub below names its current
//! exact integration gaps rather than already-landed primitives:
//!
//! | kind | what is still missing |
//! |---|---|
//! | `outrage_exact` | a second living Player — the Python branch refuses without one |
//!
//! Issue #1374 closes Inferno, Infernal Blade, Juggling, and Stomp: player-power slots fan out
//! at their owner-side hook points, the generated-card path mints physical
//! cards from the exact Generation-stream pool shuffle, Infernal Blade's
//! Energy/Star free-this-turn rows share their native lifecycle, and Stomp's
//! ordered local-cost rows are applied and cleaned at those lifecycle points.
//! Unit C (#1562) then lands typed AnyAlly identity and remote-player state;
//! issue #1561 R29 closes Blaze's checked permanent Strength writer on it;
//! issue #1936 R32 closes Demonic Shield's ordered HP-loss/current-Block
//! selected-player composite.
//!
//! The escalation rule is unchanged (PORT_PLAN §8): a refusing stub with a
//! named missing primitive is the input to the next engine slice, and a
//! guessed body is an I5 violation the differential would only find later.
//!
//! ## The card-keyword gate
//!
//! `admission::card_keywords_are_inert` still refuses retain, selections, and
//! unsupported ally-target forms. Exact Unit-C AnyAlly/AllAllies programs are
//! source-authenticated exceptions. Exhaust, X cost, AllEnemies, Power
//! routing, and Innate's already-resolved combat-entry state are modeled by
//! the surrounding engine.
//!
//! Current measurement: OUTRAGE carries no refusing keyword, so its stub is
//! its remaining gate. STOMP and INFERNAL_BLADE are modeled by their existing
//! physical-state/generation paths. BLAZE is one authenticated Unit-C AnyAlly
//! program and DEMONIC_SHIELD is a second authenticated program. The
//! escalation tests below pin the remaining split against the content tables.

use super::StepCtx;
use crate::catalog::{CardIdentity, CardTargetType, Catalog, RewardPool};
use crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109;
use crate::engine::cards::{inject_generated_free_this_turn_bottom, shuffle_generation_pool};
use crate::engine::damage::{
    alive_targets, apply_card_monster_debuff_with_catalog, apply_owner_strength,
    gain_powered_card_block, note_power, player_attack_all_from_card, player_attack_from_card,
};
use crate::engine::draw::{
    discard_and_draw, preflight_card_play_after_physical_moves,
    repair_card_play_after_physical_move,
};
use crate::engine::{EngineRefusal, Event, Subject, fire_hook};
use crate::hooks::HookEvent;
use crate::hot::{CARD_FLAG_LEGACY, HotCard, HotState, MiseryToken, PileId, RngStream};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// Seven from engine slice 2 (#1367), `exhaust_draw` and its explicit
/// second-card action choice in slice 3, Inferno/Juggling/Stomp in #1374, and
/// Blaze on Unit C in issue #1561 R29, and Demonic Shield in issue #1936 R32.
/// Outrage remains an explicit stub.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::AttackAllX,
    StepKind::AttackPerVuln,
    StepKind::BlazeExact,
    StepKind::DemonicShieldBlockExact,
    StepKind::DiscardHandDraw,
    StepKind::DrumOfBattleExact,
    StepKind::ExhaustDraw,
    StepKind::ExhaustNonattacksBlock,
    StepKind::InfernalBladeExact,
    StepKind::Inferno,
    StepKind::JugglingExact,
    StepKind::ShockwaveExact,
    StepKind::StompExact,
    StepKind::VulnerableThenStrengthCurrent,
];

/// This family's argument shapes, destructured once per body.
///
/// Since #1366 the gate holds no shape opinion on a wave kind: the body owns
/// its destructure and returns a typed refusal on surprise, which the
/// differential's `must_cover` makes visible before merge. These helpers keep
/// that reading in one place per arity.
fn one_int(ctx: &StepCtx<'_>, site: &'static str) -> Result<i64, EngineRefusal> {
    match ctx.args {
        [crate::catalog::CompiledArg::I(value)] => Ok(*value),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn two_ints(ctx: &StepCtx<'_>, site: &'static str) -> Result<(i64, i64), EngineRefusal> {
    match ctx.args {
        [
            crate::catalog::CompiledArg::I(first),
            crate::catalog::CompiledArg::I(second),
        ] => Ok((*first, *second)),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

/// `("attack_all_x", damage)` — Whirlwind.
///
/// One all-opponents attack command whose hit count is the resolved Energy-X.
/// Each hit re-resolves the living opponents
/// ([`player_attack_all_from_card`], `AttackCommand.<Execute>d__90` RVA
/// `0x3f19c0` IL_0168-IL_0196, #3023): a Phrog killed by hit N leaves its
/// Wrigglers to hit N+1.
///
/// The zero-X case is the reason this is a hit count rather than a loop the
/// body writes: OnPlay (d__6 0x40ce34) executes the command even at `x == 0`
/// (the `ble` branch skips only the VFX), and `AttackCommand.Execute`
/// (d__90 0x43ac88) still reaches `AfterAttack`, so a zero-hit command is a
/// real command that consumes Vigor. [`player_attack`] runs its outer loop
/// zero times for exactly that reason.
///
/// `card_damage` is `_powered_card_damage_base`: the enchantment, Instinct and
/// relic additives it folds are all refused content, so the row's base stands.
pub(crate) fn attack_all_x(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = one_int(ctx, "attack_all_x")?;
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        ctx.x_value,
        ctx.events,
    )
}

/// `("attack_per_vuln", base, per_stack)` — Bully.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `base = step[1] + step[2] * (target.vuln
/// if target.hp > 0 else 0)`, then one single-target, single-hit
/// `player_attack`. The Vulnerable stack count is read off the *live* target
/// before the command starts, and a dead target contributes nothing rather
/// than making the whole step a no-op.
pub(crate) fn attack_per_vuln(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (base, per_stack) = two_ints(ctx, "attack_per_vuln")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let stacks = match ctx.state.monsters.get(target) {
        Some(monster) if monster.hp > 0 => i64::from(monster.powers.value(PowerId::Vuln)),
        _ => 0,
    };
    let damage = base
        .checked_add(
            per_stack
                .checked_mul(stacks)
                .ok_or(EngineRefusal::CounterOverflow("attack_per_vuln"))?,
        )
        .ok_or(EngineRefusal::CounterOverflow("attack_per_vuln"))?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )
}

/// `blaze_exact` — exact selected-player permanent Strength.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Blaze.OnPlay` state machine RVA `0x38d0dc` reads `CardPlay.Target`, then
/// applies `StrengthPower` for the dynamic base value; canonical/upgrade RVAs
/// `0xd915c`/`0xd91c3` pin five/seven.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the exact rows,
/// resolves the stable AnyAlly key, and calls
/// `_apply_permanent_strength_to_player`. Rust uses the same
/// represented local/remote split, rehearsing the checked selected-player
/// write before any owner event or teammate scalar can publish.
pub(crate) fn blaze_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "blaze_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("blaze_exact"))?;
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("blaze_exact program"));
    };
    if ctx.spec.identity.id != CardId::Blaze
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || amount != 5 + 2 * i32::from(ctx.spec.identity.upgrade)
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::BlazeExact
        || ctx.catalog.args(program.args) != ctx.args
    {
        return Err(EngineRefusal::MalformedArgs("blaze_exact"));
    }
    let key = crate::engine::allies::target_key(ctx)?;
    let mut probe = ctx.state.clone();
    crate::engine::allies::gain_permanent_strength(&mut probe, key, amount, &mut Vec::new())?;
    crate::engine::allies::gain_permanent_strength(ctx.state, key, amount, ctx.events)
}

/// The only generated Demonic Shield programs this body accepts.
///
/// Registry equality authenticates every immutable row field, including the
/// base-only Exhaust keyword and AnyAlly target. The explicit program shape
/// prevents this helper becoming an identity-only keyword carveout if the
/// generated table changes later.
pub(crate) fn demonic_shield_program_is_exact(row: &crate::content_tables::CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::DemonicShield,
            0 | 1,
            [
                crate::content_tables::Step {
                    kind: StepKind::HpLoss,
                    args: [crate::content_tables::Arg::I(1)],
                },
                crate::content_tables::Step {
                    kind: StepKind::DemonicShieldBlockExact,
                    args: [],
                },
            ]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// `demonic_shield_block_exact` — post-loss owner Block to selected Player.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `DemonicShield/<OnPlay>d__9::MoveNext` RVA `0x3989f0` first awaits
/// owner `Damage` with base HP-loss one and props 14, then re-reads
/// `CalculatedBlock` from the owner and awaits `GainBlock` on the non-null
/// `CardPlay.Target`. Constructor/canonical/upgrade RVAs
/// `0xdd8bb`/`0xdd8f9`/`0xdd997` pin cost zero, Skill, AnyAlly, base-only
/// Exhaust, and the exact two-command program.
///
/// Python `_validate_batch281_demonic_shield_entry_spec` (frozen, deleted #2827) and
/// `_validate_batch281_demonic_shield_body` authenticate both
/// rows/programs and rehearse the whole ordered body; `_run_steps_inner` re-reads `s.block` only at this suffix and routes the powered
/// gain to the selected Player. Rust arrives here only after shared `hp_loss`
/// has committed. It authenticates that active physical source, snapshots the
/// now-live owner Block, rehearses the complete selected-player powered write
/// on a clone, and commits only after that suffix proves total. The public
/// play checkpoint restores the HP-loss prefix if this late proof refuses.
pub(crate) fn demonic_shield_block_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [loss, suffix] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("demonic_shield program"));
    };
    if !demonic_shield_program_is_exact(ctx.spec.row)
        || ctx.spec.identity.id != CardId::DemonicShield
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.spec.cost != 0
        || !ctx.spec.is_skill
        || ctx.spec.is_power
        || !ctx.spec.targeted
        || ctx.spec.target_type != CardTargetType::AnyAlly
        || ctx.spec.row.exhausts != (ctx.spec.identity.upgrade == 0)
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || loss.kind != StepKind::HpLoss
        || ctx.catalog.args(loss.args) != [crate::catalog::CompiledArg::I(1)]
        || suffix.kind != StepKind::DemonicShieldBlockExact
        || !ctx.args.is_empty()
        || ctx.catalog.args(suffix.args) != ctx.args
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
    {
        return Err(EngineRefusal::MalformedArgs("demonic_shield_block_exact"));
    }
    let Some((pile, index)) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
    else {
        return Err(EngineRefusal::MalformedArgs("demonic_shield source"));
    };
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("demonic_shield source"));
    }

    let key = crate::engine::allies::target_key(ctx)?;
    let post_loss_owner_block = i64::from(ctx.state.block);
    let mut probe = ctx.state.clone();
    crate::engine::allies::gain_powered_block(
        &mut probe,
        ctx.catalog,
        key,
        ctx.spec,
        post_loss_owner_block,
        &mut Vec::new(),
    )?;
    crate::engine::allies::gain_powered_block(
        ctx.state,
        ctx.catalog,
        key,
        ctx.spec,
        post_loss_owner_block,
        ctx.events,
    )
}

/// `("discard_hand_draw",)` — Calculated Gamble at both upgrades.
/// Native v111 OnPlay MoveNext 0x390e00 passes Hand and its frozen count to
/// DiscardAndDraw. OnUpgrade 0xda903 only adds Retain; it does not remove
/// canonical Exhaust (0xda8ad) or change this body.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). `DiscardAndDraw(hand, hand.Count)`: the
/// whole hand leaves in **hand order** (not player-chosen), then the one
/// paired Draw of that many. The count is frozen before the discards, so a
/// reshuffle triggered by the discards cannot change it.
///
/// The played instance is already in the Play pile by the time a body runs
/// (`OnPlayWrapper`, stage 3), so it is not part of the hand this empties —
/// which is why Calculated Gamble does not discard and redraw itself.
pub(crate) fn discard_hand_draw(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("discard_hand_draw"));
    }
    let hand: Vec<HotCard> = ctx.state.piles.get(PileId::Hand).as_slice().to_vec();
    let count = hand.len();
    // DiscardAndDraw RVA 0x3e0274 IL_0029–0058: once combat is over, or with
    // an empty Hand, the whole command (discards, paired Draw, Sly) is a
    // no-op. Past this gate the walk detaches each Hand card itself, in hand
    // order, and a kill mid-walk leaves the rest in Hand (#3075).
    if crate::engine::draw::card_cmd_discard_is_gated(ctx.state) || hand.is_empty() {
        return Ok(());
    }
    // Under the owning Calculated Gamble CardPlay frame the paired Draw is
    // an owned tail (#2667): the discards commit first, the Draw suspends
    // across a selecting Hellraiser child or Stratagem reshuffle with the
    // frozen count and Sly siblings parked on its record, and the Sly batch
    // replays only after the Draw fully returns. Any other caller —
    // including a foreign CardPlay owner replaying this step — keeps the
    // synchronous command, which still refuses a suspending Draw at the
    // admission gate.
    let under_own_cardplay = matches!(ctx.spec.identity.id, CardId::CalculatedGamble)
        && matches!(
            ctx.state.frames.top(),
            Some(crate::frame::Frame::CardPlay { record })
                if ctx.state.frames.card_play(record).is_some_and(|owner| {
                    owner.uid == ctx.source_uid
                        && owner.stage == crate::hot::CardPlayStage::Body
                        && !owner.pending_choice
                })
        );
    if under_own_cardplay {
        let sly = crate::engine::draw::discard_cards_collect_sly(
            ctx.state,
            ctx.catalog,
            PileId::Hand,
            &hand,
            ctx.events,
        )?;
        let paired = crate::hot::GamblePaired {
            count: u32::try_from(count)
                .map_err(|_| EngineRefusal::CounterOverflow("discard_hand_draw count"))?,
            sly: sly
                .iter()
                .map(|card| crate::hot::GambleSly {
                    uid: card.uid,
                    atom: card.atom,
                })
                .collect(),
        };
        if crate::engine::play::draw_gamble_paired_tail(ctx, paired)? {
            return Ok(());
        }
        return crate::engine::play::autoplay_sly_discard_batch(
            ctx.state,
            ctx.catalog,
            &sly,
            ctx.events,
        );
    }
    discard_and_draw(
        ctx.state,
        ctx.catalog,
        PileId::Hand,
        &hand,
        count,
        ctx.events,
    )
}

/// Calculated Gamble's paired-Draw return program (#2667).
///
/// The parked record carries the frozen discard/draw count plus the Sly
/// siblings in discard order. Each sibling must still resolve to exactly one
/// live card carrying the frozen atom — a transform reusing a projection uid
/// refuses here instead of inheriting the captured callback — and the batch
/// replays in that frozen order.
pub(crate) fn resume_gamble_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &crate::catalog::CardSpec,
    paired: &crate::hot::GamblePaired,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if gamble_program(catalog, spec).is_none() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut sly = Vec::with_capacity(paired.sly.len());
    for sibling in &paired.sly {
        let (pile, index) = crate::engine::play::unique_live_card_location(state, sibling.uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let live = state.piles.get(pile).as_slice()[index];
        if live.atom != sibling.atom {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        sly.push(live);
    }
    crate::engine::play::autoplay_sly_discard_batch(state, catalog, &sly, events)
}

/// Calculated Gamble's exact program at both upgrades: one argument-free
/// `discard_hand_draw` step. The paired Draw count is frozen from the live
/// hand at issue, so the row carries no count operand to check.
fn gamble_program(catalog: &Catalog, spec: &crate::catalog::CardSpec) -> Option<()> {
    if spec.identity.id != CardId::CalculatedGamble
        || !matches!(spec.identity.upgrade, 0 | 1)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::DiscardHandDraw && catalog.args(step.args).is_empty()).then_some(())
}

/// `("drum_of_battle_exact",)` — Drum of Battle.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827), in full: `draw_cards(s, 2, log,
/// caller="none")`. A card-body Draw, so it is a
/// [`crate::engine::draw::DrawSource::Command`] and its cards count toward
/// `non_hand_draws_this_turn`.
///
/// The card's other half is not here: exhausting a DRUM_OF_BATTLE grants
/// energy through `_drum_of_battle_after_card_exhausted`, which belongs to
/// `CardCmd.Exhaust` and lives in [`card_exhausted`].
pub(crate) fn drum_of_battle_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("drum_of_battle_exact"));
    }
    crate::engine::play::draw_cardplay_no_result(ctx, DRUM_OF_BATTLE_DRAWS)
}

/// The literal `2` of `draw_cards(s, 2, ...)`; the upgrade changes the
/// exhaust-time energy, not the draw.
const DRUM_OF_BATTLE_DRAWS: usize = 2;

/// `exhaust_draw` — Burning Pact's chosen-card exhaust followed by Draw.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) (Burning Pact). The chosen hand card
/// leaves the hand at the position the action carried,
/// `_preflight_card_exhausted` / `_card_exhausted` run, then
/// `draw_cards(s, step[1], ...)` — an ordinary command Draw.
///
/// The action vocabulary now carries the one dimension this body needs.
/// `legal_actions` (frozen Python, deleted #2827) gives a card whose first step is this kind one
/// action *per hand position*, `("play", card, pick, (pos, expos))`, deduped
/// on the remaining hand; [`crate::engine::Action::Play`] transports both
/// physical uids, so this body removes the exact selected instance.
///
/// Slice 2 landed `CardCmd.Exhaust` as [`card_exhausted`] and lifted
/// the gate's non-empty-Exhaust clause, and BURNING_PACT's own row is now
/// entirely inert to the keyword gate — its `exhausts` is false and its
/// `selects` is false, because what it exhausts is the *chosen* card and the
/// choice is not a select op.
///
/// **A one-card Hand is auto-resolved, not prompted (#3250).** v0.111.0
/// `BurningPact/<OnPlay>d__5::MoveNext` RVA `0x390434` builds its prefs with
/// `CardSelectorPrefs(ExhaustSelectionPrompt, 1)` (`IL_0032`-`IL_0038`), the
/// two-argument ctor RVA `0x1397d4` that forwards `MinSelect = MaxSelect = 1`
/// into ctor RVA `0x1397f8`, which sets `RequireManualConfirmation` to
/// `MinSelect >= 0 && MinSelect != MaxSelect` (`IL_006a`-`IL_0088`) — false.
/// `<FromHand>d__28::MoveNext` RVA `0x3e7568` then, after the zero-candidate
/// return (`IL_0152`-`IL_0161`), returns the whole candidate list without
/// consulting any selector or synchronizing a choice when
/// `!RequireManualConfirmation && Count <= MinSelect` (`IL_0167`-`IL_018d`).
/// So with exactly one other card in Hand that card is exhausted with no
/// `PlayerChoice`, whether the play is manual or auto-played. A manual play
/// already carries that card as its action's pick (the only one
/// `legal_actions` offers); an auto-play or replay reaches here with `None`,
/// and takes the sole Hand card itself.
///
/// Ending gates (#3515), each the shared IsOverOrEnding projection:
/// `CardSelectCmd.FromHand` (`BurningPact/<OnPlay>d__5` RVA `0x390434`
/// IL_004c) returns no card at `IsOverOrEnding` (`<FromHand>d__28` `0x3e7568`
/// IL_0036-003d), so nothing is exhausted, and the `CardPileCmd.Draw` after
/// the Exhaust (IL_01bc) returns at `IsOverOrEnding` too (`<DrawInternal>d__21`
/// `0x3e3a70` IL_0029-003b). An ending combat therefore makes the whole body a
/// no-op.
pub(crate) fn exhaust_draw(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let draws = one_int(ctx, "exhaust_draw")?;
    let draws =
        usize::try_from(draws).map_err(|_| EngineRefusal::CounterOverflow("exhaust_draw draws"))?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let selection = match (ctx.selection, ctx.state.piles.get(PileId::Hand).as_slice()) {
        (None, [sole]) => Some(sole.uid),
        (selection, _) => selection,
    };
    let Some(uid) = selection else {
        // Zero candidates (#2977). v0.111.0 `BurningPact/<OnPlay>d__5::
        // MoveNext` RVA `0x390434` awaits `CardSelectCmd::FromHand` with a
        // null filter (`IL_004a`-`IL_004c`). `<FromHand>d__28::MoveNext` RVA
        // `0x3e7568` builds the candidate list from the whole Hand
        // (`IL_010d`-`IL_014c`, default predicate `<>c::<FromHand>b__28_0`
        // RVA `0x3e54dd` returns true) and, when it is empty, returns it at
        // `IL_0152`-`IL_0161` without prompting. Back in the body,
        // `FirstOrDefault` is null, so `IL_00b8 brfalse.s` skips
        // `CardCmd::Exhaust` (`IL_00c4`) and falls through to the cast
        // animation and `CardPileCmd::Draw` of `Cards` (`IL_01a6`-`IL_01bc`).
        //
        // Only an empty Hand reaches here: a one-card Hand auto-resolved
        // above, and a larger one always carries a selection (the manual
        // action, or the parked Replay answer), so any other `None` stays the
        // construction error it was.
        if !ctx.state.piles.get(PileId::Hand).is_empty() {
            return Err(EngineRefusal::SelectionMismatch { required: true });
        }
        if !ctx.state.history.over {
            crate::engine::play::draw_cardplay_no_result(ctx, draws)?;
        }
        return Ok(());
    };
    let index = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .position(|card| card.uid == uid)
        .ok_or(EngineRefusal::FrozenCardVanished {
            uid,
            pile: PileId::Hand,
        })?;
    // BurningPact/<OnPlay>d__5 (RVA 0x390434) IL_00c4 awaits CardCmd.Exhaust;
    // its IsOverOrEnding gate (RVA 0x3e06c8, IL_0020–0034) leaves the card in
    // Hand, and the Draw tail below is ending-gated too (#3041).
    let Some(removed) =
        crate::engine::draw::detach_card_for_exhaust(ctx.state, PileId::Hand, index)?
    else {
        return Ok(());
    };
    let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
        crate::hot::AfterCardExhaustedReturnKind::BurningPact,
    );
    continuation.source_uid = Some(ctx.source_uid);
    let owner_live =
        crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(ctx.state, ctx.catalog);
    continuation.step_index = if owner_live {
        crate::engine::play::after_card_exhausted_source_step_cursor(ctx.state, ctx.source_uid)?
    } else {
        0
    };
    continuation.count =
        u32::try_from(draws).map_err(|_| EngineRefusal::CounterOverflow("exhaust_draw draws"))?;
    let result = crate::engine::draw::card_exhausted_with_owner(
        ctx.state,
        ctx.catalog,
        removed,
        continuation,
        ctx.events,
    )?;
    if owner_live || result == crate::engine::draw::CardExhaustedResult::Suspended {
        return Ok(());
    }
    // An Exhaust listener that kills latches `history.over`, which is the
    // Draw's IsOverOrEnding gate here; the entry gate covers every ending entry.
    if !ctx.state.history.over {
        crate::engine::play::draw_cardplay_no_result(ctx, draws)?;
    }
    Ok(())
}

/// `("exhaust_nonattacks_block", block)` — Second Wind.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Snapshot the hand's non-attacks, then
/// per card: exhaust it, then issue **its own** powered `GainBlock` command
/// from the same physical Second Wind, re-checking `s.over` between every
/// command in the pair.
///
/// Two details the snapshot carries. It is taken once, so a card drawn during
/// the loop by Dark Embrace is absent from it; and each gain is a
/// distinct command, so a per-gain modifier such as Frail or Nimble would see
/// N commands rather than one total — which is why the gain is inside the loop
/// and not a single sum at the end.
pub(crate) fn exhaust_nonattacks_block(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let block = one_int(ctx, "exhaust_nonattacks_block")?;
    let gone: Vec<HotCard> = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| {
            ctx.catalog
                .spec(card.atom)
                .is_some_and(|spec| !spec.is_attack)
        })
        .collect();
    let repairs: Vec<_> = gone
        .iter()
        .copied()
        .map(|card| (card, PileId::Hand, PileId::Exhaust))
        .collect();
    preflight_card_play_after_physical_moves(ctx.state, &repairs)?;
    continue_second_wind(
        ctx.state,
        ctx.catalog,
        ctx.source_uid,
        if crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(ctx.state, ctx.catalog)
        {
            crate::engine::play::after_card_exhausted_source_step_cursor(ctx.state, ctx.source_uid)?
        } else {
            0
        },
        i32::try_from(block).map_err(|_| EngineRefusal::CounterOverflow("Second Wind block"))?,
        &gone.iter().map(|card| card.uid).collect::<Vec<_>>(),
        ctx.events,
    )
}

/// `SecondWind/<OnPlay>d__7` RVA `0x3b9174`
/// awaits `CardCmd.Exhaust` per frozen card (IL_00dc); `<Exhaust>d__6` (`0x3e06c8`)
/// returns at `IsOverOrEnding` (IL_0025-002a) before its `CardPileCmd.Add`, so
/// an exhaust listener that ends the combat stops the walk before the next card
/// leaves Hand: the shared IsOverOrEnding projection, not `history.over`
/// (#3515).
fn continue_second_wind(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    step_index: u32,
    block: i32,
    remaining: &[u32],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let (_, source_index) = crate::engine::play::unique_live_card_location(state, source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let source_pile = crate::engine::play::unique_live_card_location(state, source_uid)?
        .ok_or(EngineRefusal::ContinuationNotModeled)?
        .0;
    let source = state.piles.get(source_pile).as_slice()[source_index];
    let spec = catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
    for (cursor, uid) in remaining.iter().copied().enumerate() {
        // The next Exhaust command is gated (IsOverOrEnding).
        if crate::engine::damage::damage_combat_is_ending(state) {
            break;
        }
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        let Some(index) = hand.iter().position(|live| live.uid == uid) else {
            // `_exact_card_index` raises when the snapshot's card is no longer
            // in hand. Nothing in this loop can move one, so reaching this is
            // a construction bug rather than a gap.
            return Err(EngineRefusal::CardNotInHand(uid));
        };
        let card = hand[index];
        let _ = hand;
        repair_card_play_after_physical_move(state, card, PileId::Hand, PileId::Exhaust)?;
        let removed = state.piles.get_mut(PileId::Hand).make_mut().remove(index);
        debug_assert_eq!(removed, card);
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(
            crate::hot::AfterCardExhaustedReturnKind::SecondWind,
        );
        continuation.source_uid = Some(source_uid);
        continuation.step_index = step_index;
        continuation.amount = block;
        continuation.remaining = remaining[cursor + 1..].to_vec();
        let owner_live =
            crate::engine::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog);
        let result = crate::engine::draw::card_exhausted_with_owner(
            state,
            catalog,
            removed,
            continuation,
            events,
        )?;
        if owner_live || result == crate::engine::draw::CardExhaustedResult::Suspended {
            return Ok(());
        }
        // `if s.over: break` — the GainBlock command is gated too.
        if state.history.over {
            break;
        }
        gain_powered_card_block(state, catalog, spec, i64::from(block), events)?;
        fire_hook(catalog, HookEvent::AfterBlockGained, state, events)?;
    }
    Ok(())
}

pub(crate) fn resume_second_wind_after_exhaust(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let source_uid = record
        .source_uid
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if record.flags == 0 && !state.history.over {
        let mut after_block = record.clone();
        after_block.flags = 1;
        state
            .frames
            .replace_top_after_card_exhausted_power(&after_block)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let (pile, index) = crate::engine::play::unique_live_card_location(state, source_uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[index];
        let spec = catalog
            .spec(source.atom)
            .ok_or(EngineRefusal::UnknownAtom(source.atom))?;
        gain_powered_card_block(state, catalog, spec, i64::from(record.amount), events)?;
        fire_hook(catalog, HookEvent::AfterBlockGained, state, events)?;
        if !matches!(
            state.frames.top(),
            Some(crate::frame::Frame::AfterCardExhaustedPower { .. })
        ) {
            return Ok(());
        }
    }
    let completed = state
        .frames
        .pop_top_after_card_exhausted_power()
        .filter(|completed| {
            completed.return_kind == crate::hot::AfterCardExhaustedReturnKind::SecondWind
                && completed.card_uid == record.card_uid
                && (state.history.over || completed.flags == 1)
        })
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if state.history.over {
        return Ok(());
    }
    continue_second_wind(
        state,
        catalog,
        source_uid,
        completed.step_index,
        completed.amount,
        &completed.remaining,
        events,
    )
}

/// `infernal_blade_exact` — full generation-pool shuffle, then one fresh L0
/// Attack made free until turn end or play.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) delegates to `add_infernal_blade_card`, which shuffles the 33-card Ironclad attack pool on the
/// `generation` stream (`UnstableShuffle` consumes 33 - 1 = 32 draws), makes
/// the drawn canonical free this turn, and adds it to Hand/Bottom or
/// Discard/Bottom when the hand is full.
///
pub(crate) fn infernal_blade_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::InfernalBlade
        || ctx.state.reward_card_pool != Some(RewardPool::Ironclad)
        || ctx.state.entropy_card_pool != Some(RewardPool::Ironclad)
        || ctx.state.rng.is_vacant(RngStream::Generation)
    {
        return Err(EngineRefusal::MalformedArgs("infernal_blade_exact"));
    }

    // Catalog preview makes the deterministic leaf immutable and admission
    // walks it. Re-run the whole mutation on a clone so any construction
    // regression remains an atomic refusal on the real state.
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply_infernal_blade(&mut probe, ctx.catalog, &mut probe_events)?;
    apply_infernal_blade(ctx.state, ctx.catalog, ctx.events)
}

fn apply_infernal_blade(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let shuffled = shuffle_generation_pool(state, &INFERNAL_BLADE_ATTACK_POOL_V109)?;
    inject_generated_free_this_turn_bottom(
        state,
        catalog,
        CardIdentity {
            id: shuffled[0],
            upgrade: 0,
            enchantment: None,
        },
        PileId::Hand,
        events,
    )
}

/// `("inferno", amount)` — Inferno.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827): `_validated_hibernate_amount(s)`, refuse
/// if any unkeyed `AfterPlayerTurnStart` peer is active, then
/// `s.inferno += step[1]` and `s.inferno_self += 1`.
///
/// The amount stacks permanently, while the private self-damage counter gains
/// one per application. Owner-side HP-loss sites read the public amount and
/// the turn-start tail reads the private counter.
///
/// `Inferno/<OnPlay>d__3` RVA `0x3a72b8` awaits `PowerCmd.Apply<InfernoPower>`
/// (IL_00d1) and increments the private counter only on the non-null result
/// (IL_0131-0134). `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn inferno(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "inferno")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("inferno"))?;
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("inferno"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let inferno = ctx
        .state
        .powers
        .value(PowerId::Inferno)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("inferno"))?;
    let self_damage = ctx
        .state
        .powers
        .value(PowerId::InfernoSelf)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("inferno_self"))?;
    if ctx.state.powers.value(PowerId::Inferno) <= 0
        && !ctx.state.register_after_player_turn_start(PowerId::Inferno)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-player-turn-start listener order",
        ));
    }
    for (power, value) in [
        (PowerId::Inferno, inferno),
        (PowerId::InfernoSelf, self_damage),
    ] {
        ctx.state.powers.set(power, SlotWire::Int, value);
        note_power(ctx.events, Subject::Player, power, value);
    }
    Ok(())
}

/// `("juggling_exact", amount)` — Juggling.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827): under a `_W197_CARD_FIELDS` drift guard,
/// seed `s.juggling_attacks` from `s.owner_attack_plays_started_this_turn` on
/// the **first** application only — the stacking path modifies the amount and
/// skips `AfterApplied`, so it must not reseed — then `s.juggling += step[1]`.
///
/// The first application seeds the private attack count from owner play-start
/// history; stacking changes only the public amount. `play` advances that
/// private count before each attack body and mints clones exactly at three.
///
/// v0.111.0 `Juggling/<OnPlay>` RVA `0x3a84f8` awaits `PowerCmd.Apply<JugglingPower>` at IL_00c1.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn juggling_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount: i32 = one_int(ctx, "juggling_exact")?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("juggling_exact"))?;
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("juggling_exact"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let current = ctx.state.powers.value(PowerId::Juggling);
    if current < 0 {
        return Err(EngineRefusal::CounterOverflow("juggling"));
    }
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("juggling"))?;
    crate::engine::turn::prepare_after_side_turn_end_scalar_write(
        ctx.state,
        PowerId::Juggling,
        current,
        updated,
    )?;
    if current == 0 {
        let seed = i32::from(ctx.state.history.owner_attack_plays_started_this_turn);
        ctx.state
            .powers
            .set(PowerId::JugglingAttacks, SlotWire::Int, seed);
        if seed != 0 {
            note_power(ctx.events, Subject::Player, PowerId::JugglingAttacks, seed);
        }
    }
    ctx.state
        .powers
        .set(PowerId::Juggling, SlotWire::Int, updated);
    note_power(ctx.events, Subject::Player, PowerId::Juggling, updated);
    Ok(())
}

/// `outrage_exact` — not modeled, and not *reachable* in a one-player fight.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) delegates to `_apply_outrage_exact`: a whole-body clone preflight, the local single-target attack,
/// then one fresh owner-retargeted source clone into every living Player's
/// Discard/Bottom.
///
/// Escalated: **a second living Player**. Batch 1b sharpened the batch-1
/// citation here rather than restating it, because
/// `_validated_batch289_outrage_recipients` (frozen Python, deleted #2827) *raises* when the living
/// recipient list has one entry — OUTRAGE is MultiplayerOnly. Every fight this
/// engine admits has exactly one player, so a faithful port of this branch
/// would refuse on every reachable state. Filling the body would be strictly
/// worse than the stub: OUTRAGE's rows are inert to the keyword gate, so
/// listing the kind would **admit** the entry and move a certain refusal from
/// setup into mid-play, which is the failure the escalation rule exists to
/// prevent.
///
/// If a second player ever exists, the machinery is unchanged: the ordered
/// roster (#1361), generated source clones (#1362; `_commit_outrage_clones`),
/// the `_active_play_card` identity read, and the whole-body preflight on a
/// cloned state. The batch-1 citation's **0-cost-attack line is closed** —
/// slice 2 carries `zero_energy_attack_plays_started_this_turn` (#1314).
pub(crate) fn outrage_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::OutrageExact))
}

/// `("shockwave_exact", amount)` — Shockwave.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). One identity snapshot of `s.alive()`,
/// then **target-major** Weak → Vulnerable: each recipient takes both debuffs
/// before the next recipient takes any, and each independently awaited Apply
/// re-checks `s.over` and the recipient's life.
///
/// The `break` in Python's inner loop is per recipient, not per command — a
/// recipient that died between its Weak and its Vulnerable is skipped for the
/// Vulnerable and the walk moves on to the next creature.
pub(crate) fn shockwave_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "shockwave_exact")?;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("shockwave_exact"))?;
    for target in alive_targets(ctx.state) {
        for (power, token) in [
            (PowerId::Weak, MiseryToken::Weak),
            (PowerId::Vuln, MiseryToken::Vuln),
        ] {
            if ctx.state.history.over || ctx.state.monsters[target].hp <= 0 {
                break;
            }
            apply_card_monster_debuff_with_catalog(
                ctx.state,
                ctx.catalog,
                target,
                power,
                token,
                amount,
                ctx.events,
            )?;
        }
    }
    Ok(())
}

/// `("stomp_exact", damage)` — Stomp.
///
/// One powered all-opponents attack command that hits each living opponent
/// once ([`player_attack_all_from_card`]). The card's separate
/// `BeforeCardPlayed` listener lives in `engine::play`: every owner Attack
/// appends one `-1` this-turn local-cost row to every live Stomp before this
/// body runs.
pub(crate) fn stomp_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = one_int(ctx, "stomp_exact")?;
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        1,
        ctx.events,
    )
}

/// `("vulnerable_then_strength_current", amount)` — Dominate.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). One `if not s.over and target.hp > 0`
/// gate over the whole body, the Vulnerable application, then — behind a
/// second `if not s.over` — `_apply_owner_strength(s, target.vuln, ...)`.
///
/// The Strength amount is a **fresh** read of the target's live `vuln` rather
/// than the amount just applied, which is Python's own note on the branch:
/// `GetPower<VulnerablePower>` runs after the awaited application, and Artifact
/// can eat that application and leave the previous total standing.
///
/// DOMINATE's Exhaust is the play pipeline's result routing, not this body's
/// work. (This comment described the kind as escalated on player powers until
/// batch 1b replaced it: slice 2 ported the body and listed the kind in the
/// same diff, but left the stub's notice above it.)
///
/// `Dominate/<OnPlay>d__8` RVA `0x39995c` awaits `Apply<VulnerablePower>`
/// (IL_00ee) and then `Apply<StrengthPower>` on the owner (IL_018d). `<Apply>d__1`1`
/// (`0x3ef988`) returns at `IsEnding` (IL_0025-002a), and `apply_owner_strength`
/// still reads `history.over`, so the Strength tests the shared IsEnding
/// projection (#3515).
pub(crate) fn vulnerable_then_strength_current(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = one_int(ctx, "vulnerable_then_strength_current")?;
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("vulnerable_then_strength_current"))?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if ctx.state.history.over || ctx.state.monsters[target].hp <= 0 {
        return Ok(());
    }
    apply_card_monster_debuff_with_catalog(
        ctx.state,
        ctx.catalog,
        target,
        PowerId::Vuln,
        MiseryToken::Vuln,
        amount,
        ctx.events,
    )?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    // A **fresh** read after the awaited application, not the amount just
    // applied: Artifact can eat the application and leave the old total
    // standing, and the Strength the player gains is that live total.
    let stacks = ctx.state.monsters[target].powers.value(PowerId::Vuln);
    apply_owner_strength(ctx.state, stacks, ctx.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{Catalog, CatalogBuilder};
    use crate::content_tables::{card_row, card_rows};
    use crate::engine::capability_manifest;
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotState, LocalCostExpiration, PileId, RngStreamState,
    };
    use crate::ids::CardId;
    use crate::rng::Xoshiro256StarStar;

    /// The R0.5 slice fixture, loaded through the ordinary boundary.
    fn fixture() -> (HotState, Catalog) {
        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .expect("the checked-in fixture parses");
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (state, catalog)
    }

    #[test]
    fn blaze_authenticates_both_rows_and_routes_one_checked_permanent_write() {
        for upgrade in 0..=1 {
            let identity = CardIdentity {
                id: CardId::Blaze,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            let atom = builder.intern(identity).unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let args = catalog.args(step.args);
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.multiplayer_ally_key = 1;
            state
                .fanouts
                .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                    key: 1,
                    ..crate::hot::MultiplayerAllyState::default()
                });
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: Some(1),
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            blaze_exact(&mut ctx).unwrap();
            assert_eq!(
                ctx.state.fanouts.multiplayer_ally().strength,
                5 + 2 * i32::from(upgrade)
            );
            assert_eq!(ctx.state.powers.value(PowerId::Strength), 0);

            ctx.target = Some(0);
            blaze_exact(&mut ctx).unwrap();
            assert_eq!(
                ctx.state.powers.value(PowerId::Strength),
                5 + 2 * i32::from(upgrade)
            );
        }
    }

    #[test]
    fn demonic_shield_refuses_forged_program_args_target_and_active_source_atomically() {
        let identity = CardIdentity {
            id: CardId::DemonicShield,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let blaze_atom = builder
            .intern(CardIdentity {
                id: CardId::Blaze,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let suffix = catalog.steps(&spec)[1];
        let source = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        let exact_state = || {
            let mut state = HotState::at_defaults();
            state.hp = 20;
            state.block = 5;
            state.multiplayer_ally_key = 1;
            state
                .fanouts
                .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                    key: 1,
                    ..crate::hot::MultiplayerAllyState::default()
                });
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            state
        };

        for (name, source_uid, target, args, active_uid) in [
            (
                "nonempty args",
                7,
                Some(0),
                &[crate::catalog::CompiledArg::I(1)][..],
                Some(7),
            ),
            ("missing target", 7, None, &[][..], Some(7)),
            ("forged target", 7, Some(2), &[][..], Some(7)),
            ("inactive source", 7, Some(0), &[][..], None),
            ("different active source", 7, Some(0), &[][..], Some(8)),
            ("different source uid", 8, Some(0), &[][..], Some(8)),
        ] {
            let mut state = exact_state();
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid,
                target,
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            let mut invoke = || demonic_shield_block_exact(&mut ctx);
            let result = match active_uid {
                Some(uid) => crate::engine::play::with_test_active_play(uid, invoke),
                None => invoke(),
            };
            assert!(result.is_err(), "{name}");
            assert_eq!(state, before, "{name}");
            assert!(events.is_empty(), "{name}");
        }

        for name in ["forged row", "forged program"] {
            let mut forged_catalog = catalog.clone();
            match name {
                "forged row" => {
                    forged_catalog.spec_mut_for_test(atom).unwrap().row =
                        card_row(CardId::Blaze, 0).unwrap();
                }
                "forged program" => {
                    let forged = forged_catalog.spec(blaze_atom).unwrap().steps;
                    forged_catalog.spec_mut_for_test(atom).unwrap().steps = forged;
                }
                _ => unreachable!(),
            }
            let forged_spec = *forged_catalog.spec(atom).unwrap();
            let mut state = exact_state();
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &forged_catalog,
                spec: &forged_spec,
                source_uid: 7,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: forged_catalog.args(suffix.args),
                events: &mut events,
            };
            let result = crate::engine::play::with_test_active_play(7, || {
                demonic_shield_block_exact(&mut ctx)
            });
            assert!(result.is_err(), "{name}");
            assert_eq!(state, before, "{name}");
            assert!(events.is_empty(), "{name}");
        }

        let mut legacy = exact_state();
        legacy.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_LEGACY;
        let before = legacy.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut legacy,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: catalog.args(suffix.args),
            events: &mut events,
        };
        assert!(
            crate::engine::play::with_test_active_play(7, || {
                demonic_shield_block_exact(&mut ctx)
            })
            .is_err()
        );
        assert_eq!(legacy, before);
        assert!(events.is_empty());
    }

    #[test]
    fn infernal_blade_consumes_a_full_shuffle_and_mints_a_free_exact_card() {
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.next_card_uid = 41;

        let mut preview = state.clone();
        let selected =
            shuffle_generation_pool(&mut preview, &INFERNAL_BLADE_ATTACK_POOL_V109).unwrap()[0];
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::InfernalBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: selected,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };

        infernal_blade_exact(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Generation).counter, 32);
        assert!(state.exact_piles);
        let generated = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(generated.uid, 41);
        assert_ne!(generated.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(catalog.spec(generated.atom).unwrap().identity.id, selected);
        let instance = state.card_states.get(generated.uid);
        let rows = instance.local_cost_modifiers;
        assert_eq!(rows.resolve(99), 0);
        assert_eq!(
            rows.as_slice()[0].expiration,
            LocalCostExpiration::ThisTurnOrPlayed
        );
        assert_eq!(instance.free_star_cost_this_turn_or_played_rows, 1);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
    }

    #[test]
    fn ended_infernal_blade_shuffles_and_records_before_skipping_the_add() {
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.history.over = true;
        state.history.owner_generated_cards_combat = 3;
        state.next_generated_hook_uid = 5;
        state.next_card_uid = 41;

        let mut preview = state.clone();
        let selected =
            shuffle_generation_pool(&mut preview, &INFERNAL_BLADE_ATTACK_POOL_V109).unwrap()[0];
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::InfernalBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: selected,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };

        infernal_blade_exact(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Generation).counter, 32);
        assert_eq!(state.history.owner_generated_cards_combat, 4);
        assert_eq!(state.next_generated_hook_uid, 6);
        assert_eq!(state.next_card_uid, 41);
        assert!(!state.exact_piles);
        assert!(state.card_states.get(41).is_vacant());
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert!(events.is_empty());
    }

    /// Every kind this file owns a body for, ported or not.
    ///
    /// The dispatch's own completeness and uniqueness are already pinned by
    /// `tests/hot_path_contract.rs`; what this list adds is the split between
    /// claimed and escalated, so that porting a kind means moving it into
    /// [`IMPLEMENTED`] in one deliberate edit.
    const OWNED: [StepKind; 15] = [
        StepKind::AttackAllX,
        StepKind::AttackPerVuln,
        StepKind::BlazeExact,
        StepKind::DemonicShieldBlockExact,
        StepKind::DiscardHandDraw,
        StepKind::DrumOfBattleExact,
        StepKind::ExhaustDraw,
        StepKind::ExhaustNonattacksBlock,
        StepKind::InfernalBladeExact,
        StepKind::Inferno,
        StepKind::JugglingExact,
        StepKind::OutrageExact,
        StepKind::ShockwaveExact,
        StepKind::StompExact,
        StepKind::VulnerableThenStrengthCurrent,
    ];

    /// The kinds still waiting on an engine primitive, re-derived against this
    /// tree by batch 1b (#1326), less `exhaust_draw`, which slice 3 closes.
    const ESCALATED: [StepKind; 1] = [StepKind::OutrageExact];

    /// The wave's honesty property, from this side: the manifest claims
    /// nothing this file cannot actually play, and claims everything it can.
    #[test]
    fn the_manifest_claims_nothing_this_family_has_not_ported() {
        let manifest = capability_manifest();
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
    /// unmodeled card play as if the step were absent, which is the exact I5
    /// failure the escalation rule exists to prevent — and the reviewer's
    /// checklist item this pins mechanically. Routing is proved on the way:
    /// only this file's bodies name these kinds.
    #[test]
    fn every_escalated_stub_refuses_by_its_own_kind() {
        let (mut state, catalog) = fixture();
        let spec = *catalog
            .spec(state.piles.get(PileId::Hand).as_slice()[0].atom)
            .expect("the fixture's hand cards are interned");
        for kind in ESCALATED {
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
                crate::steps::apply_step(kind, &mut ctx),
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

    /// The card each escalated kind gates, one per kind for this family.
    const ESCALATION_CARDS: [(StepKind, CardId); 1] = [(StepKind::OutrageExact, CardId::Outrage)];

    /// The escalations are facts about this tree, not prose — so pin them.
    ///
    /// Every stub above says *why* it still refuses. Batch 1b found several of
    /// those reasons had silently expired: slice 2 cleared a clause of a
    /// refusal without clearing the refusal, and nothing went red, so the
    /// stubs kept pointing the next engine slice at primitives that already
    /// existed. This test is the fix for that failure mode rather than for any
    /// one citation. It asserts the split the module header describes, so a
    /// slice that clears one of these clauses reddens *this file* — whose
    /// comments are then the thing to correct — instead of leaving the wave's
    /// work list quietly wrong.
    #[test]
    fn the_escalation_reasons_still_hold() {
        for (kind, id) in ESCALATION_CARDS {
            let rows = card_rows(id);
            assert!(!rows.is_empty(), "{:?} has no rows", kind.as_str());
            for row in rows {
                assert!(
                    row.steps.iter().any(|step| step.kind == kind),
                    "{} no longer carries {:?}",
                    row.name,
                    kind.as_str()
                );
            }
        }

        // These formerly escalated rows remain useful metadata guards even
        // though their exact bodies and keyword routes are now implemented.
        for row in card_rows(CardId::Inferno)
            .iter()
            .chain(card_rows(CardId::Juggling))
        {
            assert!(row.is_power, "{} is no longer a Power", row.name);
        }
        assert!(
            card_row(CardId::Juggling, 1)
                .expect("JUGGLING+1 is a row")
                .innate,
            "JUGGLING+1 lost the innate clause its stub cites"
        );

        // OUTRAGE is keyword-inert, so its stub remains the entry's only gate.
        for row in card_rows(CardId::Outrage) {
            assert!(
                row.playable
                    && !row.is_power
                    && !row.innate
                    && !row.retain
                    && !row.sly
                    && !row.selects
                    && !row.ethereal
                    && !row.x_cost
                    && row.cost >= 0
                    && matches!(row.target_type, "Self" | "AnyEnemy" | "AllEnemies"),
                "{} grew a keyword clause its stub does not cite",
                row.name
            );
        }
    }

    #[test]
    fn burning_pact_pick_stays_in_hand_once_combat_is_over() {
        // #3041: BurningPact/<OnPlay>d__5 (0x390434) IL_00c4 awaits
        // CardCmd.Exhaust; its IsOverOrEnding gate (0x3e06c8 IL_0020–0034)
        // returns before Add detaches the card, and the Draw tail is gated.
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::BurningPact,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let step = catalog.steps(&spec)[0];
        assert_eq!(step.kind, StepKind::ExhaustDraw);
        let pick = HotCard {
            uid: 1,
            atom: defend,
            flags: 0,
        };
        let draw = HotCard {
            uid: 2,
            atom: defend,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().push(pick);
        state.piles.get_mut(PileId::Draw).make_mut().push(draw);
        state.history.over = true;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: Some(pick.uid),
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        exhaust_draw(&mut ctx).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[pick]);
        assert_eq!(state.piles.get(PileId::Draw).as_slice(), &[draw]);
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    /// #3250: a `None` selection over a one-card Hand is `FromHand`'s
    /// non-manual auto-resolve (`<FromHand>d__28` RVA `0x3e7568`
    /// IL_0167-IL_018d) and exhausts that card; over two cards it is still
    /// the construction error, because `Count` 2 exceeds `MinSelect` 1.
    #[test]
    fn burning_pact_none_selection_takes_a_sole_hand_card_and_refuses_two() {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::BurningPact,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(source_atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let card = |uid| HotCard {
            uid,
            atom: defend,
            flags: 0,
        };
        for hand in [vec![card(1)], vec![card(1), card(2)]] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .extend(hand.iter().copied());
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 7,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };
            let result = exhaust_draw(&mut ctx);
            if hand.len() == 1 {
                result.unwrap();
                assert!(state.piles.get(PileId::Hand).is_empty());
                assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), &[card(1)]);
                assert_eq!(state.history.owner_cards_exhausted_combat, 1);
            } else {
                assert_eq!(
                    result,
                    Err(EngineRefusal::SelectionMismatch { required: true })
                );
                assert_eq!(state.piles.get(PileId::Hand).as_slice(), &hand[..]);
            }
        }
    }

    #[test]
    fn calculated_gamble_leaves_the_hand_once_combat_is_over() {
        // #3075: CalculatedGamble OnPlay (0x390e00) passes Hand to
        // CardCmd.DiscardAndDraw, whose IsOverOrEnding entry gate (0x3e0274
        // IL_0029-0035) makes the discards, paired Draw and Sly pass a no-op.
        // Before, the Hand was cleared first and vanished.
        let mut builder = CatalogBuilder::new();
        let gamble_atom = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::CalculatedGamble,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(crate::catalog::CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(gamble_atom).unwrap();
        let step = catalog.steps(&spec)[0];
        assert_eq!(step.kind, StepKind::DiscardHandDraw);
        let hand = [1, 2].map(|uid| HotCard {
            uid,
            atom: defend,
            flags: 0,
        });
        let draw = HotCard {
            uid: 3,
            atom: defend,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend(hand);
        state.piles.get_mut(PileId::Draw).make_mut().push(draw);
        state.history.over = true;
        let before = state.clone();
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 7,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        discard_hand_draw(&mut ctx).unwrap();

        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    fn plain_identity(id: CardId) -> crate::catalog::CardIdentity {
        crate::catalog::CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        }
    }

    /// #3515: each body gates on its native command, not `history.over`,
    /// and is a no-op while the combat is ending before the over latch; the
    /// Adaptable-vetoed control writes:
    /// - Burning Pact's `CardSelectCmd.FromHand` (`<FromHand>d__28` 0x3e7568
    ///   IL_0036, `IsOverOrEnding`): nothing is exhausted or drawn;
    /// - Second Wind's per-card `CardCmd.Exhaust` (`<Exhaust>d__6` 0x3e06c8
    ///   IL_0025, `IsOverOrEnding`);
    /// - Inferno's and Juggling's `PowerCmd.Apply`, and Dominate's owner
    ///   `Apply<StrengthPower>` (`<Apply>d__1`1` 0x3ef988 IL_0025, `IsEnding`).
    #[test]
    fn ironclad_uncommon_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        use crate::catalog::CompiledArg;
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::BurningPact,
            CardId::SecondWind,
            CardId::Inferno,
            CardId::Juggling,
            CardId::Dominate,
            CardId::DefendIronclad,
        ] {
            builder.intern(plain_identity(id)).unwrap();
        }
        let catalog = builder.build();
        let spec = |id| {
            *catalog
                .spec(catalog.atom(&plain_identity(id)).unwrap())
                .unwrap()
        };
        let defend = catalog
            .atom(&plain_identity(CardId::DefendIronclad))
            .unwrap();
        let card = |uid| HotCard {
            uid,
            atom: defend,
            flags: 0,
        };
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1));
        template
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([card(2), card(3)]);
        template
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom: catalog.atom(&plain_identity(CardId::SecondWind)).unwrap(),
                flags: 0,
            });
        type Body = fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>;
        let rows: [(CardId, Body, &[CompiledArg], Option<u32>); 5] = [
            (
                CardId::BurningPact,
                exhaust_draw,
                &[CompiledArg::I(2)],
                Some(1),
            ),
            (
                CardId::SecondWind,
                exhaust_nonattacks_block,
                &[CompiledArg::I(5)],
                None,
            ),
            (CardId::Inferno, inferno, &[CompiledArg::I(6)], None),
            (CardId::Juggling, juggling_exact, &[CompiledArg::I(1)], None),
            (
                CardId::Dominate,
                vulnerable_then_strength_current,
                &[CompiledArg::I(1)],
                None,
            ),
        ];
        for (id, body, args, selection) in rows {
            let spec = spec(id);
            crate::engine::damage::assert_ending_window_gate(
                &template,
                &format!("{id:?}"),
                |s, t| {
                    body(&mut StepCtx {
                        state: s,
                        catalog: &catalog,
                        spec: &spec,
                        source_uid: 7,
                        target: Some(t),
                        selection,
                        x_value: 0,
                        args,
                        events: &mut Vec::new(),
                    })
                },
            );
        }
        // Dominate's owner Strength reads the target's live Vulnerable, so the
        // target must already carry some for the gated write to exist.
        let dominate = spec(CardId::Dominate);
        for vetoed in [false, true] {
            let mut state = template.clone();
            let target = crate::engine::damage::push_ending_window_roster(&mut state, vetoed);
            state.monsters_mut()[target].powers.set(
                crate::ids::PowerId::Vuln,
                crate::powers::SlotWire::Int,
                2,
            );
            vulnerable_then_strength_current(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &dominate,
                source_uid: 7,
                target: Some(target),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(1)],
                events: &mut Vec::new(),
            })
            .unwrap();
            assert_eq!(
                state.powers.value(crate::ids::PowerId::Strength),
                if vetoed { 3 } else { 0 },
                "Dominate vetoed={vetoed}"
            );
        }
    }
}
