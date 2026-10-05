//! Card-step bodies for the `content/cards/regent_uncommon.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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

use super::StepCtx;
use crate::catalog::{CardIdentity, CardSpec, Catalog, CompiledArg, RewardPool};
use crate::content_tables::{Arg, LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101};
#[cfg(test)]
use crate::engine::cards::shuffle_generation_pool;
use crate::engine::cards::{inject_generated_exact_bottom, shuffle_generation_slice};
use crate::engine::damage::gain_powered_card_block;
use crate::engine::{EngineRefusal, Event, Subject};
use crate::hot::{CARD_FLAG_LEGACY, HotCard, HotState, PileId, RngStream};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

#[cfg(test)]
thread_local! {
    static MONOLOGUE_COLD_CARD_ENTRY_CHECKS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_monologue_cold_card_entry_checks() {
    MONOLOGUE_COLD_CARD_ENTRY_CHECKS.with(|checks| checks.set(0));
}

#[cfg(test)]
pub(crate) fn monologue_cold_card_entry_checks() -> usize {
    MONOLOGUE_COLD_CARD_ENTRY_CHECKS.with(std::cell::Cell::get)
}

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// **All eleven kinds are implemented.** Issue #1374 landed Bundle of
/// Joy's exact Generation-stream shuffle and fresh-card insertion, and the
/// Manifest Authority card slice composes those same primitives with the
/// powered-Block funnel. Each remaining stub below names only what is still
/// missing or records that it belongs to a separate card slice rather than
/// silently broadening #1374. Unit C #1562 adds the exact two-Player quotient
/// rooted in `_resolve_ally_target_key` (frozen Python, deleted #2827) and lands
/// Huddle Up plus Plot. Issue #1362 closes Quasar through the existing
/// body-owned selection frame and Largesse through Unit C's stable Player key
/// plus its one missing Colorless-unlock provenance bit.
///
/// The owner-local generation seam is now closed: catalog construction
/// pre-interns the deterministic leaf set reachable from the canonical
/// Generation stream (the shuffle-prefix walk for exhausting sources, the
/// whole pool for Manifest Authority's unbounded replays — see
/// `boundary::intern_generation_preview`), and `engine::cards` performs the
/// native full-pool Fisher-Yates plus singular history/mint/hand-cap
/// transactions. Quasar and Largesse both reuse that same gate rather than
/// growing a second shuffle or mint path.
///
/// The batch-1 wave blocker is gone: `admission::step_args` went body-owned
/// in #1366, so a filled body's argument shape is its own to validate and
/// nothing ahead of this family refuses on shape. The keyword gate's residue,
/// re-measured against the generated rows: exhaust is now inert (so both
/// BUNDLE_OF_JOY rows and both MANIFEST_AUTHORITY rows are keyword-clean).
/// Monologue now owns both exact rows, including L1's native Retain lifecycle.
/// R53A closes Tutor's exact two-Player target and owner-local physical-uid
/// selection surface without extending the multiplayer quotient.
/// Void Form owns its exact static-Ethereal L0 lifecycle. Constellation's exact fixed-Star and
/// `AnyAlly` surfaces, Quasar's fixed-Star and body-owned selection surfaces,
/// and Largesse's exact `AnyAlly` program are now closed. A state holding a
/// MANIFEST_AUTHORITY copy still refuses at admission today — not on this
/// body, but on its whole-pool leaf closure, many of whose members are
/// independently refused cards — so the body below is exercised by its unit tests and
/// becomes differentially reachable as the pool's leaves land.
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::BundleOfJoyExact,
    StepKind::ConstellationExact,
    StepKind::GuidingStarDrawNextTurnExact,
    StepKind::HuddleUpExact,
    StepKind::LargesseExact,
    StepKind::ManifestAuthorityExact,
    StepKind::Monologue,
    StepKind::PlotExact,
    StepKind::QuasarExact,
    StepKind::TutorExact,
    StepKind::VoidForm,
];

/// `("bundle_of_joy_exact", count)` — one full Colorless-pool shuffle,
/// followed by three/four singular fresh-card inserts.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) → `_apply_bundle_of_joy_exact`,
/// which is `_regent_colorless_generation_shuffle(s, count)` into
/// `_add_fresh_generated_cards_to_hand` with singular per-card
/// combat-ending gates.
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`):
/// `BundleOfJoy/<OnPlay>d__5::MoveNext` RVA `0x390188` loads
/// `CardPool<ColorlessCardPool>` (IL_0023), `Owner.UnlockState` (IL_002e) and
/// `CardMultiplayerConstraint` (IL_0039) into `GetUnlockedCards` (IL_003e),
/// passes that list with `DynamicVars.Cards` straight to
/// `GetDistinctForCombat(.., CombatCardGeneration)` (IL_0068), and awaits
/// one `AddGeneratedCardToCombat(Hand, Bottom)` per result (IL_0095). The pool
/// is `Catalog::colorless_generation_pool` under the recorded profile, and no
/// owner class is read (#2560), so any owner that passes
/// [`colorless_body_provenance_is_exact`] draws it.
pub(crate) fn bundle_of_joy_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected = i64::from(3 + ctx.spec.identity.upgrade);
    if ctx.args != [CompiledArg::I(expected)]
        || ctx.spec.identity.id != CardId::BundleOfJoy
        || !colorless_body_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("bundle_of_joy_exact"));
    }
    let count = usize::try_from(expected)
        .map_err(|_| EngineRefusal::MalformedArgs("bundle_of_joy_exact"))?;
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply_bundle_of_joy(&mut probe, ctx.catalog, count, &mut probe_events)?;
    apply_bundle_of_joy(ctx.state, ctx.catalog, count, ctx.events)
}

/// The gate Bundle of Joy and Manifest Authority share (#2560).
///
/// [`crate::engine::cards::colorless_generation_provenance_is_exact`] — an
/// owner, an absent-or-agreeing `entropy_card_pool`, a RECORDED profile,
/// solo, and a live Generation stream — plus the Regent conjunct both bodies
/// always carried: a Regent document must publish its
/// `spectrum_shift_generation_pool`, the guard `catalog`'s "The Regent
/// conjunct" section keeps in step with Discovery and Entropy. Neither body
/// reads the owner's class (IL above), so the old `reward_card_pool ==
/// Regent` / `entropy_card_pool == Regent` equalities are gone: an Entropy
/// transform can mint either card for any owner.
pub(crate) fn colorless_body_provenance_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    crate::engine::cards::colorless_generation_provenance_is_exact(state, catalog)
        && (state.reward_card_pool != Some(RewardPool::Regent)
            || state.spectrum_shift_generation_pool())
}

fn apply_bundle_of_joy(
    state: &mut HotState,
    catalog: &Catalog,
    count: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let pool = catalog.colorless_generation_pool();
    let shuffled = shuffle_generation_slice(state, &pool)?;
    for id in shuffled.iter().copied().take(count) {
        inject_generated_exact_bottom(
            state,
            catalog,
            CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            },
            1,
            PileId::Hand,
            events,
        )?;
    }
    Ok(())
}

/// `("constellation_exact", draw, energy, block)` — selected Player Draw,
/// Energy, then powered Block.
///
/// Current v0.111.0 IL (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// pins `Constellation/<OnPlay>d__11::MoveNext` at RVA `0x394148`. It captures
/// `cardPlay.Target.Player` once, then separately awaits `CardPileCmd.Draw`,
/// `PlayerCmd.GainEnergy`, and powered `CreatureCmd.GainBlock` against that
/// same Player, in that order. Constructor/canonical-var/upgrade RVAs
/// `0xdbb13`/`0xdbb26`/`0xdbbbb` pin zero Energy, two fixed Stars, 1 Draw,
/// 1 Energy, and 9/12 Block.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) authenticates the live CardPlay frame,
/// then `_apply_constellation_sequence` clone-rehearses the complete
/// selected-player Draw → Energy → powered Block sequence. The Rust body uses
/// Unit C's stable Player key from `_resolve_ally_target_key` and the
/// same complete rehearsal. Command NoDraw and hand capacity suppress only
/// Draw; remote NoEnergyGain suppresses only Energy; a terminal Draw
/// suppresses both later commands.
pub(crate) fn constellation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("constellation_exact action"));
    }
    let (draw, energy, block) = constellation_program(ctx.catalog, ctx.spec)
        .ok_or(EngineRefusal::MalformedArgs("constellation_exact program"))?;
    if ctx.args
        != [
            CompiledArg::I(i64::try_from(draw).expect("one Draw fits i64")),
            CompiledArg::I(i64::from(energy)),
            CompiledArg::I(i64::from(block)),
        ]
    {
        return Err(EngineRefusal::MalformedArgs("constellation_exact operands"));
    }
    preflight_constellation_exact(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec, ctx.target)?;
    let key = constellation_target_key(ctx.state, ctx.target)?;
    if key == 0
        && matches!(
            ctx.state.frames.top(),
            Some(crate::frame::Frame::CardPlay { .. })
        )
    {
        let parked = crate::engine::play::draw_cardplay_owned_tail(ctx, draw)?;
        if parked {
            return Ok(());
        }
        return apply_constellation_tail(
            ctx.state,
            ctx.catalog,
            ctx.spec,
            key,
            energy,
            block,
            ctx.events,
        );
    }
    apply_constellation_sequence(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        key,
        draw,
        energy,
        block,
        ctx.events,
    )
}

fn constellation_program(catalog: &Catalog, spec: &CardSpec) -> Option<(usize, i32, i32)> {
    let expected = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::Constellation, 0) => (1, 1, 9),
        (CardId::Constellation, 1) => (1, 1, 12),
        _ => return None,
    };
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::ConstellationExact
        && catalog.args(step.args)
            == [
                CompiledArg::I(i64::try_from(expected.0).ok()?),
                CompiledArg::I(i64::from(expected.1)),
                CompiledArg::I(i64::from(expected.2)),
            ])
    .then_some(expected)
}

fn exact_constellation_source(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    let matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice())
        .filter(|card| card.uid == source_uid)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: matches.len(),
        });
    }
    let source = matches[0];
    if source.flags & CARD_FLAG_LEGACY != 0 || catalog.spec(source.atom) != Some(spec) {
        return Err(EngineRefusal::MalformedArgs(
            "constellation_exact physical source",
        ));
    }
    Ok(())
}

fn constellation_target_key(state: &HotState, target: Option<usize>) -> Result<u32, EngineRefusal> {
    let key: u32 = target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("Constellation Player key"))?;
    let live = match key {
        0 => state.hp > 0,
        1 => state.multiplayer_ally_key == 1 && state.fanouts.multiplayer_ally().alive,
        _ => false,
    };
    if !live {
        return Err(EngineRefusal::BadTarget(
            u8::try_from(key).unwrap_or(u8::MAX),
        ));
    }
    Ok(key)
}

/// Rehearse the complete awaited sequence before a public play publishes its
/// resource/source/event prefix. Shared play integration calls this while the
/// physical source is still in its authenticated caller-owned pile.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn preflight_constellation_exact(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    target: Option<usize>,
) -> Result<(), EngineRefusal> {
    let (draw, energy, block) = constellation_program(catalog, spec)
        .ok_or(EngineRefusal::MalformedArgs("constellation_exact program"))?;
    exact_constellation_source(state, catalog, source_uid, spec)?;
    let key = constellation_target_key(state, target)?;
    let mut probe = state.clone();
    if key == 0
        && matches!(
            probe.frames.top(),
            Some(crate::frame::Frame::CardPlay { .. })
        )
    {
        let [step] = catalog.steps(spec) else {
            return Err(EngineRefusal::MalformedArgs("constellation_exact program"));
        };
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut probe,
            catalog,
            spec,
            source_uid,
            target,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        let parked = crate::engine::play::draw_cardplay_owned_tail(&mut ctx, draw)?;
        if parked {
            return Ok(());
        }
        return apply_constellation_tail(
            &mut probe,
            catalog,
            spec,
            key,
            energy,
            block,
            &mut events,
        );
    }
    // The public CardPlay transaction supplies the persistent child carrier.
    // This cold preflight separately proves the ordinary Draw machinery and
    // the Energy/powered-Block tail. Hellraiser's exact selecting child is
    // authenticated by the rooted Draw/CardPlay grammar; current selector
    // completion cannot increase Energy or Dexterity (Tender only decreases
    // Dexterity), so suppressing that one clone-only hook cannot hide a later
    // overflow or powered-Block refusal.
    if key == 0 && state.powers.value(PowerId::Hellraiser) > 0 {
        probe.powers.set(PowerId::Hellraiser, SlotWire::Int, 0);
    }
    apply_constellation_sequence(
        &mut probe,
        catalog,
        spec,
        key,
        draw,
        energy,
        block,
        &mut Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_constellation_sequence(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    key: u32,
    draw: usize,
    energy: i32,
    block: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // `Constellation/<OnPlay>d__11::MoveNext` RVA `0x394148` has no card-level
    // ending check: it awaits `CardPileCmd::Draw` (IL_0170), whose
    // `<DrawInternal>d__21::MoveNext` RVA `0x3e3a70` returns at
    // `CombatManager::get_IsOverOrEnding` (IL_002e-003b) before
    // `Hook::ShouldDraw`. The Draw gate is that projection (#3112).
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    crate::engine::allies::draw_for_player(state, catalog, key, draw, events)?;
    apply_constellation_tail(state, catalog, spec, key, energy, block, events)
}

/// Constellation's Energy and Block commands, each behind its own native gate.
///
/// `PlayerCmd::GainEnergy` (IL_01e9) is gated inside `allies::gain_energy`
/// (#2708). `CreatureCmd::GainBlock` (IL_0263) is
/// `CreatureCmd/<GainBlock>d__18::MoveNext` RVA `0x3eaec0`, which returns at
/// `CombatManager::get_IsOverOrEnding` (IL_0032-0041) before the dead-creature
/// check and any Block write, so the Block gate is the IsOverOrEnding
/// projection rather than `history.over` (#3112).
fn apply_constellation_tail(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    key: u32,
    energy: i32,
    block: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    crate::engine::allies::gain_energy(state, key, energy)?;
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    crate::engine::allies::gain_powered_block(state, catalog, key, spec, i64::from(block), events)
}

pub(crate) fn resume_constellation_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    target: Option<usize>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (_, energy, block) =
        constellation_program(catalog, spec).ok_or(EngineRefusal::ContinuationNotModeled)?;
    let key = constellation_target_key(state, target)?;
    if key != 0 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    apply_constellation_tail(state, catalog, spec, key, energy, block, events)
}

/// `("guiding_star_draw_next_turn_exact", amount)` — Guiding Star's delayed
/// draw application.
///
/// Current v0.111.0 IL (assembly SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`) pins
/// `GuidingStar/<OnPlay>d__6::MoveNext` at RVA `0x3a2c70`: after the ordinary
/// powered `FromCard` attack, the body applies DrawCardsNextTurnPower 2/3 to
/// the owner. Constructor/star/variable RVAs `0xe19fb`/`0xe1a08`/`0xe1a0b`
/// pin cost one, fixed Star cost one, damage 12/13, and delayed draw 2/3.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) validates the complete current row
/// against `_B294_PRIVATE_CARD_SPECS`, then increments `s.draw_next_turn`
/// only while combat remains live. The Rust row identity check is the same
/// generated-table handoff. `engine::play` owns the preceding physical-UID
/// attack and Energy-then-Star spend; `allies::add_draw_next_turn` writes the
/// represented owner slot, and `engine::turn` consumes and removes that slot
/// before the next hand draw. Admission refuses the only currently
/// unrepresented Power-application listener order and the maximum +3
/// overflow before either manual play or AutoPlay can start.
pub(crate) fn guiding_star_draw_next_turn_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<(), EngineRefusal> {
    let amount = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::GuidingStar, 0, [CompiledArg::I(2)]) => 2,
        (CardId::GuidingStar, 1, [CompiledArg::I(3)]) => 3,
        _ => {
            return Err(EngineRefusal::MalformedArgs(
                "guiding_star_draw_next_turn_exact",
            ));
        }
    };
    if crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
        != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs(
            "guiding_star_draw_next_turn_exact source row",
        ));
    }
    if ctx.target.is_none() {
        return Err(EngineRefusal::TargetMismatch { required: true });
    }
    crate::engine::allies::add_draw_next_turn(ctx.state, 0, amount)
}

/// `("huddle_up_exact", amount)` — exact ordered Player Draw fan-out (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) → `_apply_huddle_up_draws`,
/// an ordered living-player Draw fan-out over
/// `_validated_batch285_huddle_up_recipients`.
///
/// The remote quotient owns its exact Draw/Hand/Discard order and independent
/// Shuffle stream. Admission refuses all remote draw/shuffle listeners and
/// payloads whose draw effect is not represented before either recipient
/// mutates.
pub(crate) fn huddle_up_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let amount = huddle_up_program(ctx.catalog, ctx.spec)
        .filter(|amount| ctx.args == [CompiledArg::I(i64::try_from(*amount).unwrap_or(-1))])
        .ok_or(EngineRefusal::MalformedArgs("huddle_up_exact"))?;
    preflight_huddle_up_exact(ctx, amount)?;
    if ctx.state.hp > 0
        && matches!(
            ctx.state.frames.top(),
            Some(crate::frame::Frame::CardPlay { .. })
        )
    {
        if crate::engine::play::draw_cardplay_owned_tail(ctx, amount)? {
            return Ok(());
        }
        return huddle_up_remote_suffix(ctx.state, ctx.catalog, amount, ctx.events);
    }
    huddle_up_sequence(ctx.state, ctx.catalog, amount, ctx.events)
}

/// `HuddleUp/<OnPlay>d__7` RVA `0x3a60bc` draws for each living teammate
/// through `CardPileCmd.DrawWithoutBlockingOnOtherPlayers` (IL_0097, which
/// awaits `CardPileCmd.Draw`, `0x13142c` IL_0027). `<DrawInternal>d__21`
/// (`0x3e3a70`) returns at `IsOverOrEnding` (IL_0029-003b), so each recipient
/// tests the shared IsOverOrEnding projection, not `history.over` (#3515).
fn huddle_up_sequence(
    state: &mut HotState,
    catalog: &Catalog,
    amount: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let recipients = crate::engine::allies::living_keys(state).collect::<Vec<_>>();
    for key in recipients {
        if crate::engine::damage::damage_combat_is_ending(state) {
            break;
        }
        crate::engine::allies::draw_for_player(state, catalog, key, amount, events)?;
    }
    Ok(())
}

fn huddle_up_program(catalog: &Catalog, spec: &CardSpec) -> Option<usize> {
    let amount = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::HuddleUp, 0) => 2,
        (CardId::HuddleUp, 1) => 3,
        _ => return None,
    };
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::HuddleUpExact
        && catalog.args(step.args) == [CompiledArg::I(i64::try_from(amount).ok()?)])
    .then_some(amount)
}

fn preflight_huddle_up_exact(ctx: &StepCtx<'_>, amount: usize) -> Result<(), EngineRefusal> {
    let mut probe = ctx.state.clone();
    let local_live = ctx.state.hp > 0;
    if local_live
        && matches!(
            probe.frames.top(),
            Some(crate::frame::Frame::CardPlay { .. })
        )
    {
        let mut events = Vec::new();
        let mut probe_ctx = StepCtx {
            state: &mut probe,
            catalog: ctx.catalog,
            spec: ctx.spec,
            source_uid: ctx.source_uid,
            target: ctx.target,
            selection: ctx.selection,
            x_value: ctx.x_value,
            args: ctx.args,
            events: &mut events,
        };
        let parked = crate::engine::play::draw_cardplay_owned_tail(&mut probe_ctx, amount)?;
        if parked {
            return Ok(());
        }
        huddle_up_remote_suffix(&mut probe, ctx.catalog, amount, &mut events)?;
    } else {
        if local_live && probe.powers.value(PowerId::Hellraiser) > 0 {
            // As for Constellation, the public CardPlay owns any selecting
            // local child. The clone proves the fixed local Draw plus remote
            // suffix without executing that child synchronously.
            probe.powers.set(PowerId::Hellraiser, SlotWire::Int, 0);
        }
        huddle_up_sequence(&mut probe, ctx.catalog, amount, &mut Vec::new())?;
    }
    Ok(())
}

/// `HuddleUp/<OnPlay>d__7` RVA `0x3a60bc` draws for each living teammate
/// through `CardPileCmd.DrawWithoutBlockingOnOtherPlayers` (IL_0097, which
/// awaits `CardPileCmd.Draw`, `0x13142c` IL_0027). `<DrawInternal>d__21`
/// (`0x3e3a70`) returns at `IsOverOrEnding` (IL_0029-003b). The remote Draw
/// (`allies::remote_draw`) carries that gate itself (#3515).
fn huddle_up_remote_suffix(
    state: &mut HotState,
    catalog: &Catalog,
    amount: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    if state.multiplayer_ally_key == 1 && state.fanouts.multiplayer_ally().alive {
        crate::engine::allies::draw_for_player(state, catalog, 1, amount, events)?;
    }
    Ok(())
}

pub(crate) fn resume_huddle_up_after_local_draw(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let [step] = catalog.steps(spec) else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    let [CompiledArg::I(amount)] = catalog.args(step.args) else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    huddle_up_remote_suffix(
        state,
        catalog,
        usize::try_from(*amount).map_err(|_| EngineRefusal::ContinuationNotModeled)?,
        events,
    )
}

/// `("largesse_exact",)` — selected-Player pool, owner RNG and owner insert.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) resolves the stable
/// target through `_resolve_ally_target_key`, then
/// `_apply_largesse_exact` reads that Player's exact epochs while
/// retaining the source owner's mutable generation transaction. Its caller
/// `_apply_action_impl` uniquely keeps LARGESSE in the apply-time
/// MultiplayerOnly set even though `_card_can_play` omits it: solo
/// can offer the card, but attempting the action must refuse before mutation.
///
/// Current v0.111.0 IL (assembly SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`) pins
/// `Largesse/<OnPlay>d__3::MoveNext` at RVA `0x3a90e4`: it reads the exact
/// `cardPlay.Target.Player` Colorless pool/unlock state, calls
/// `CardFactory::GetDistinctForCombat` (`0x112878`) with the source owner's
/// CombatCardGeneration stream, optionally upgrades the one result, then
/// awaits `CardPileCmd::AddGeneratedCardToCombat` (`0x1305a0`) on the source
/// owner. The shared `UnstableShuffle` at `0x1131e4` therefore consumes 61
/// draws over the complete 62-row current-build multiplayer Colorless pool.
///
/// The pool is keyed by the TARGET Player's unlocks (#3285, re-read at
/// `0x3a90e4`: IL_00b3-IL_00f7 load `cardPlay.Target.Player`,
/// `CardPool<ColorlessCardPool>` and that Player's `UnlockState` into
/// `GetUnlockedCards`; IL_00fc-IL_0112 then pass the SOURCE owner's
/// `CombatCardGeneration` to `GetDistinctForCombat`). A local target (key 0)
/// therefore draws [`crate::steps::neutral::largesse_colorless_pool`] under
/// the local recorded profile (`Catalog::largesse_local_colorless_pool`), so a
/// partial local profile shuffles its own shorter pool rather than refusing.
/// A remote target still needs its exact fully-unlocked Colorless epochs
/// (`fully_unlocked_colorless_epochs`): the boundary refuses any other remote
/// profile at parse, so a remote pool is always the frozen 62 rows.
pub(crate) fn largesse_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Largesse
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || ctx.state.reward_card_pool != Some(RewardPool::Regent)
        || ctx.state.rng.is_vacant(RngStream::Generation)
    {
        return Err(EngineRefusal::MalformedArgs("largesse_exact"));
    }
    if crate::engine::allies::living_keys(ctx.state).count() <= 1 {
        return Err(EngineRefusal::CardNotPlayableSolo(CardId::Largesse));
    }
    let target = ctx
        .target
        .and_then(|target| u32::try_from(target).ok())
        .ok_or(EngineRefusal::MalformedArgs("largesse_exact target"))?;
    if !crate::engine::allies::living_keys(ctx.state).any(|key| key == target)
        || (target == 0
            && !crate::engine::cards::unlock_profile_is_recorded(ctx.state, ctx.catalog))
        || (target != 0
            && !ctx
                .state
                .fanouts
                .multiplayer_ally()
                .fully_unlocked_colorless_epochs)
    {
        return Err(EngineRefusal::MalformedArgs("largesse_exact target pool"));
    }
    let pool = if target == 0 {
        ctx.catalog.largesse_local_colorless_pool()
    } else {
        LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101.to_vec()
    };
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs("largesse_exact target pool"));
    }
    fn body(
        state: &mut HotState,
        catalog: &Catalog,
        pool: &[CardId],
        upgrade: u8,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        if state.history.over {
            return Ok(());
        }
        let shuffled = crate::engine::cards::shuffle_generation_slice(state, pool)?;
        inject_generated_exact_bottom(
            state,
            catalog,
            CardIdentity {
                id: shuffled[0],
                upgrade,
                enchantment: None,
            },
            1,
            PileId::Hand,
            events,
        )
    }
    let upgrade = ctx.spec.identity.upgrade;
    let mut probe = ctx.state.clone();
    body(&mut probe, ctx.catalog, &pool, upgrade, &mut Vec::new())?;
    body(ctx.state, ctx.catalog, &pool, upgrade, ctx.events)
}

/// `("manifest_authority_exact", block)` — powered Block, then one exact
/// Colorless generation.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) → `_apply_manifest_authority_exact`: `_gain_powered_card_block` first, then one
/// `_regent_colorless_generation_shuffle(s, 1)` — a full 50-card shuffle
/// whose selected prefix is a single fresh card — into
/// `_add_fresh_generated_cards_to_hand` with singular per-card
/// combat-ending gates. An upgraded source raises the fresh card to L1
/// (`card_upgrade_to(generated[0], 1)`) before insertion; the fresh tuple
/// carries no physical state, so the upgrade is a pure identity raise.
///
/// The order is load-bearing: the Block gain precedes the shuffle, and the
/// shuffle advances the Generation stream even when the fight has already
/// ended (both the Block funnel's entry gate and the generated-add's leading
/// gate no-op on `over`, but neither guards the stream).
///
/// Unlike Bundle of Joy this card does not exhaust, so a copy can return
/// through a reshuffle and mint again: its reachable leaf set is the whole
/// 50-card pool, which `boundary::intern_generation_preview` interns in full
/// and the admission walk therefore vets in full. The differential's probe
/// rows currently prove a justified whole-pool refusal rather than play
/// parity; the unit tests below carry the body's evidence until the pool's
/// remaining leaves land.
///
/// Current v0.111.0 IL (`sts2.dll` SHA-256 `9cb4f1ad…12b4`):
/// `ManifestAuthority/<OnPlay>d__7::MoveNext` RVA `0x3ab860` awaits
/// `CreatureCmd.GainBlock` (IL_0041) first, then loads
/// `CardPool<ColorlessCardPool>` (IL_00a2), `Owner.UnlockState` (IL_00ad) and
/// `CardMultiplayerConstraint` (IL_00bd) into `GetUnlockedCards` (IL_00c2),
/// `GetDistinctForCombat(.., 1, CombatCardGeneration)` (IL_00dd), upgrades the
/// result when the source is upgraded (IL_00f5), and awaits one
/// `AddGeneratedCardToCombat(Hand, Bottom)` (IL_0103). The pool is
/// `Catalog::colorless_generation_pool` under the recorded profile (#2560),
/// and no owner class is read.
pub(crate) fn manifest_authority_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected = i64::from(7 + ctx.spec.identity.upgrade);
    if ctx.args != [CompiledArg::I(expected)]
        || ctx.spec.identity.id != CardId::ManifestAuthority
        || !colorless_body_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("manifest_authority_exact"));
    }
    let generated_upgrade = if ctx.spec.identity.upgrade != 0 { 1 } else { 0 };
    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply_manifest_authority(
        &mut probe,
        ctx.catalog,
        ctx.spec,
        expected,
        generated_upgrade,
        &mut probe_events,
    )?;
    apply_manifest_authority(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        expected,
        generated_upgrade,
        ctx.events,
    )
}

fn apply_manifest_authority(
    state: &mut HotState,
    catalog: &Catalog,
    source: &crate::catalog::CardSpec,
    block: i64,
    generated_upgrade: u8,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    gain_powered_card_block(state, catalog, source, block, events)?;
    let pool = catalog.colorless_generation_pool();
    let shuffled = shuffle_generation_slice(state, &pool)?;
    if shuffled.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "manifest_authority_exact pool",
        ));
    }
    inject_generated_exact_bottom(
        state,
        catalog,
        CardIdentity {
            id: shuffled[0],
            upgrade: generated_upgrade,
            enchantment: None,
        },
        1,
        PileId::Hand,
        events,
    )
}

/// `("monologue", 1)` — exact MonologuePower application.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The write increments `s.monologue`
/// behind `_validated_monologue_state` — but that validator reads a
/// *pair*, and the second half is what the power is for.
///
/// Current v0.111.0 ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Monologue/<OnPlay>d__6::MoveNext` RVA `0x3adebc` applies one fresh
/// instance and writes the returned power's Strength dynamic variable to
/// the card's exact Power value, one. `MonologuePower::BeforeCardPlayed` RVA
/// `0xa4b6c` freezes that dynamic Strength for the exact physical card;
/// `AfterCardPlayed` RVA `0x33e660` removes the key, applies the frozen
/// Strength, then increments that object's `StrengthApplied`; and owner
/// `AfterSideTurnEnd` RVA `0x33e7d8` removes that instance before reversing
/// its own ledger. `MonologuePower::get_InstanceType` RVA `0xa4b15` returns
/// Instanced. The shared play/selection/turn foundation preserves
/// those callbacks at `_monologue_after_card_played_preflight` (frozen Python, deleted #2827) and
/// `_dispatch_after_side_turn_end_power`, and rolls back any
/// late refusal as one public action.
pub(crate) fn monologue(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    monologue_foundation_exact(ctx)
}

fn monologue_program_is_exact(spec: &CardSpec, catalog: &Catalog) -> bool {
    matches!(
        (spec.identity.id, spec.identity.upgrade),
        (CardId::Monologue, 0 | 1)
    ) && crate::engine::play::body_enchantment_is_exact(spec)
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
        && matches!(
            spec.row.steps,
            [crate::content_tables::Step {
                kind: StepKind::Monologue,
                args: [Arg::I(1)]
            }]
        )
        && matches!(
            catalog.steps(spec),
            [step]
                if step.kind == StepKind::Monologue
                    && catalog.args(step.args) == [CompiledArg::I(1)]
        )
}

/// Authenticate Monologue's deliberately private per-instance quotient.
///
/// Current-v0.111 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `MonologuePower::get_CanonicalVars` RVA `0xa4b18` exposes positive Int32
/// Amount plus nonnegative `StrengthApplied`; its listener set is exactly
/// `BeforeCardPlayed` (`0xa4b6c`), `AfterCardPlayed` (`0x33e660`), and
/// `AfterSideTurnEnd` (`0x33e7d8`). The cold rows are execution authority;
/// legacy Amount, ledger, and hook bits remain checked aggregate mirrors.
pub(crate) fn monologue_private_state_is_exact(state: &HotState) -> bool {
    let amount = match state.powers.get(PowerId::Monologue) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value > 0 => slot.value,
        Some(_) => return false,
    };
    let Some(row_amount) = state
        .fanouts
        .monologue_instances()
        .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
    else {
        return false;
    };
    state.fanouts.instanced_player_power_records_are_exact()
        && state.fanouts.monologue_hook_flags_are_exact()
        && amount == row_amount
        && ((amount == 0 && !state.fanouts.monologue_hooks_are_registered())
            || (amount > 0
                && state.fanouts.monologue_hooks_are_registered()
                && crate::engine::damage::player_type_one_listener_order_is_exact(state)))
}

/// Whether a public play/resume or turn action can reach Monologue's late
/// listener suffix and therefore needs one shallow-COW transaction.
pub(crate) fn monologue_listener_is_reachable(state: &HotState) -> bool {
    state.powers.get(PowerId::Monologue).is_some()
        || state.fanouts.monologue_strength_applied() != 0
        || state.fanouts.monologue_hooks_are_registered()
        || !state.fanouts.monologue_hook_flags_are_exact()
        || state
            .pending
            .as_deref()
            .and_then(|pending| pending.record(&state.frames))
            .is_some_and(|record| record.monologue)
}

/// Cold active-only half of the per-body dictionary-key authentication.
///
/// Public play/resume entry validates every orphan/partial private shape once.
/// An inactive ordinary body therefore never scans the physical piles or the
/// execution stack. The first replay after Monologue itself registers still
/// enters this helper, as does every already-live Monologue body. Ball's
/// current-frame reader rejects a suspended parent or an outer recursive play
/// with the same uid.
#[cold]
#[inline(never)]
pub(crate) fn monologue_active_card_play_entry_is_exact(state: &HotState, card: HotCard) -> bool {
    #[cfg(test)]
    MONOLOGUE_COLD_CARD_ENTRY_CHECKS.with(|checks| checks.set(checks.get() + 1));
    monologue_private_state_is_exact(state)
        && crate::engine::play::active_card_current_context(card.uid)
            .is_some_and(|(play_index, _)| play_index.is_some())
        && crate::engine::play::unique_live_card_location(state, card.uid).is_ok_and(|location| {
            location.is_some_and(|(pile, index)| state.piles.get(pile).as_slice()[index] == card)
        })
}

/// Exact private owner-side-end subset: the authenticated per-instance
/// quotient with no still-unread peer beside it. Turn entry reads the two
/// halves apart so each refuses under its own name.
#[cfg(test)]
pub(crate) fn monologue_side_end_entry_is_exact(state: &HotState) -> bool {
    monologue_private_state_is_exact(state) && !monologue_side_end_peer_is_unmodeled(state)
}

/// Whether a player power whose owner-side-end body has not been read beside
/// Monologue's is live at turn end (#3434).
///
/// Both bodies run from the one acquisition-ordered `AfterSideTurnEnd` object
/// ledger (`engine::turn::dispatch_after_side_turn_end_power`), so a peer is
/// placed exactly where it was acquired. Retain Hand was the first peer read.
/// Current v0.111.0 ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `MonologuePower/<AfterSideTurnEnd>d__17::MoveNext` RVA `0x33e7d8` leaves
/// unless `participants` holds its owner (IL_0024-IL_0037), removes itself
/// (`PowerCmd::Remove`, IL_003c-IL_003d), then applies minus its own
/// `StrengthApplied` to Strength (IL_0097-IL_00c5).
/// `RetainHandPower/<AfterSideTurnEnd>d__5::MoveNext` RVA `0x342694` takes the
/// same owner guard (IL_001d-IL_0030) and only decrements itself
/// (`PowerCmd::Decrement`, IL_0032-IL_0033); `RetainHandPower::ShouldFlush`
/// RVA `0xa6a8c` reads the owner's player alone. Neither body reads the
/// other's object, Strength, or the hand, so the two commute and the ledger's
/// order is the whole interaction. `MonologuePower::BeforeCardPlayed` RVA
/// `0xa4b6c` and `<AfterCardPlayed>d__16::MoveNext` RVA `0x33e660` compare
/// only `CardPlay.Card.Owner` with the power's owner (IL_000c-IL_0022,
/// IL_0020-IL_003d): how the card came to be played, by hand or by an
/// AutoPlay, is not read anywhere in the power.
///
///
/// # The dispatch order (#3613)
///
/// `Hook/<AfterSideTurnEnd>d__81::MoveNext` RVA `0x3d11c0` walks
/// `Hook::IterateCombatHookListeners` (IL_0058) and awaits each listener's
/// `AfterSideTurnEnd` to its completion or its player-choice pause before
/// starting the next (IL_00b0-IL_00bd); no body read here asks for a choice,
/// so each finishes before the next starts. `CombatState/<IterateHookListeners>d__69::MoveNext` RVA
/// `0x3f9720` adds each creature's `Creature::get_Powers` whole (IL_0092-
/// IL_0097), and that list grows only by `Creature::ApplyPowerInternal` RVA
/// `0x11da0c` IL_0064-IL_006f `List.Add`: acquisition order, which is the
/// ledger's. `Hook/<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`
/// tests `IsOverOrEnding` once, before the walk (IL_0028-IL_0042); the inner
/// iterator then tests `CombatState::Contains` per listener as it is reached
/// (IL_02a6), and for a power that is `Owner.Player.IsActiveForHooks`
/// (`CombatState::Contains` RVA `0x137564` IL_00ae-IL_00d1).
///
/// # The peers read here (#3613)
///
/// Each takes Monologue's owner guard and then touches only itself, or itself
/// and one stat:
///
/// * **No Draw**, `NoDrawPower/<AfterSideTurnEnd>d__5::MoveNext` RVA
///   `0x33efbc`: guard IL_001d-IL_002e, `PowerCmd::Remove` IL_0032-IL_0033.
/// * **Shadowmeld**, `ShadowmeldPower/<AfterSideTurnEnd>d__7::MoveNext` RVA
///   `0x344114`: guard IL_001d-IL_002e, `PowerCmd::Remove` IL_0032-IL_0033.
/// * **Double Damage**, `DoubleDamagePower/<AfterSideTurnEnd>d__5::MoveNext`
///   RVA `0x33983c`: guard IL_001d-IL_002e, `PowerCmd::Decrement`
///   IL_0032-IL_0033.
/// * **Temporary Dexterity**,
///   `TemporaryDexterityPower/<AfterSideTurnEnd>d__22::MoveNext` RVA
///   `0x348498`: guard IL_0025-IL_0035, `PowerCmd::Remove` IL_0043, then
///   `Apply<DexterityPower>` of minus `Sign * Amount` (IL_00aa-IL_00c4).
/// * **Temporary Strength**,
///   `TemporaryStrengthPower/<AfterSideTurnEnd>d__22::MoveNext` RVA
///   `0x348ba8`: guard IL_0024-IL_0035, `PowerCmd::Remove` IL_0043, then
///   `Apply<StrengthPower>` of minus `Sign * Amount` (IL_00aa-IL_00c4).
///
/// `PowerCmd/<Remove>d__8::MoveNext` RVA `0x3f09bc` is `RemoveInternal`
/// (IL_0030) and the removed power's own `AfterRemoved` (IL_00b4): it raises
/// no hook another power hears, and neither these classes nor the wrapper
/// subclasses Rust carries (`FeedingFrenzyPower`, `SetupStrikePower`,
/// `AnticipatePower`, `SpeedPotionPower`, `FadePower`: an `OriginModel`
/// getter and a constructor each) override `AfterRemoved`.
/// `PowerCmd/<Decrement>d__4::MoveNext` RVA `0x3f025c` is `ModifyAmount`
/// (IL_0029). Monologue implements none of the power-amount hooks that
/// `ModifyAmount` and `Apply` raise (its listener set is the three in
/// [`monologue_private_state_is_exact`]). Among the peers the only one is the
/// wrapper's own
/// `Temporary{Strength,Dexterity}Power/<AfterPowerAmountChanged>d__21`
/// (RVAs `0x348a88`, `0x348378`), which returns unless the changed power is
/// itself (IL_003d-IL_0046), so Monologue's Strength write does not reach it.
///
/// So the first four share no state with Monologue's body, and commute with
/// it. Temporary Strength shares Strength: natively both bodies subtract from
/// the one `StrengthPower`, and subtraction commutes, with nothing reading
/// Strength between the two. Rust holds the wrapper's part in
/// `HotState::temp_strength`, apart from the Strength slot Monologue's
/// reversal writes, so its arm clears the wrapper and Monologue's lowers the
/// slot, and the two cannot meet. An intermediate Strength of exactly zero
/// removes and re-creates the native `StrengthPower` in one order and not the
/// other. Its place in the owner's list is not represented for the player,
/// as it already is not for the wrapper beside Ritual or Tender, and
/// `StrengthPower` is no ordered listener.
///
/// No body read above, Monologue's and Retain Hand's included, can end the
/// combat or kill the owner. The two that can are read next.
///
/// # Doom and Consuming Shadow (#3628)
///
/// Both compose with Monologue through the ledger's order alone. Each cell
/// below was run on the live engine (headless harness, v0.111.0 `41cef1ea`,
/// seed `PROBE3628`) in both acquisition orders.
///
/// **Doom.** `DoomPower/<AfterSideTurnEnd>d__9::MoveNext` RVA `0x3390f0`
/// leaves on the enemy side (IL_001d-IL_0026) and unless
/// `ShouldDoomTrigger` RVA `0xa1a00` holds (IL_002b-IL_0039: not
/// `IsOverOrEnding`, the owner among `participants`, alive, doomed and the
/// first doomed creature of its side), then awaits `DoomKill`
/// (IL_003b-IL_0047). `<DoomKill>d__6::MoveNext` RVA `0x3392c8` is
/// `CreatureCmd::Kill(creature, false)` per doomed creature (IL_00d8-IL_00df)
/// and `Hook::AfterDiedToDoom` (IL_017d). Monologue's body reads neither HP
/// nor Doom, and Doom's reads neither Monologue nor Strength, so the order
/// matters only through the owner's life:
///
/// * *Doom does not trigger.* Its body does nothing; Monologue's runs as it
///   does alone, in either order.
/// * *Lethal, Monologue acquired first.* Monologue removes itself and
///   reverses its Strength, then Doom kills.
/// * *Lethal, Doom acquired first.*
///   `CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext` RVA
///   `0x3ebe90` runs `RemoveAllPowersAfterDeath` (IL_04f1) and
///   `Player::DeactivateHooks` (IL_072d). The listener walk is a lazy
///   iterator that tests `CombatState::Contains` per listener as it is
///   reached (`0x3f9720` IL_02a3-IL_02ab; `Contains` RVA `0x137564`
///   IL_00ba-IL_00d1 is `Owner.Player.IsActiveForHooks` for a power), so
///   Monologue's body is never called: no Strength Apply is recorded. The
///   ledger walk skips every row once the player's hooks are deactivated
///   (`engine::turn::drive_after_side_turn_end_power_listeners`), which is
///   that test. The corpse keeps its Monologue object, hooks, ledger row and
///   Strength, as it keeps every other power a death does not clear in Rust.
/// * *Prevented* (`Hook::ShouldDie` false at IL_0301-IL_030b, then
///   `Hook::AfterPreventingDeath` IL_085d; Rust admits Fairy in a Bottle and
///   Lizard Tail). The owner is alive with active hooks and the combat is not
///   ending, so Monologue's body runs whole in either order, and Doom stays.
///
/// **Consuming Shadow.** `ConsumingShadowPower/<AfterSideTurnEnd>d__4`
/// RVA `0x3375a4` takes the owner guard (IL_0027-IL_003a), leaves on an empty
/// orb queue (IL_003f-IL_0060) and awaits `OrbCmd::EvokeLast` once per live
/// Amount (IL_0071-IL_0083, back-edge IL_014c-IL_0158). An evoke that leaves
/// the combat going shares nothing with Monologue's body. One that kills the
/// last enemy leaves the owner's hooks active, so a Monologue acquired later
/// is still called: `PowerCmd/<Remove>d__8::MoveNext` RVA `0x3f09bc` has no
/// combat gate (null test IL_0023, `RemoveInternal` IL_0030), and the
/// reversal stops at `PowerCmd/<Apply>d__1`1::MoveNext` RVA `0x3ef988`
/// IL_0020-IL_002a `IsEnding`. Native ends with Monologue gone and its
/// Strength still applied. The Monologue arm does the same under
/// `history.over`, which the evoke's killing blow latches in the same
/// command (#3515), so the arm sees the ending the evoke made. A Monologue
/// acquired first has already reversed.
///
/// The window in which `history.over` is not native's `IsEnding` (#3515) is a
/// state that enters the turn end already ending and not over. Native then walks
/// no listener at all (`Hook/<IterateCombatHookListeners>d__0` `0x3d3bc0`
/// IL_0028-IL_0042). That is every row's question, not this pair's, and
/// belongs to #3521.
///
/// # Still refused, each for its own reason
///
/// * A pending **Dark Embrace** ethereal tally draws from the ledger and can
///   park mid-walk (#3496).
/// * The player's `PowerId::TempStrength` **slot** has no writer and no
///   ledger row (the player's wrapper is `HotState::temp_strength`), so a
///   nonzero value is a state nothing here produced.
pub(crate) fn monologue_side_end_peer_is_unmodeled(state: &HotState) -> bool {
    state.powers.value(PowerId::TempStrength) != 0 || state.fanouts.dark_embrace_ethereal() != 0
}

/// Exact public Monologue writer. Admission authenticates the generated row,
/// canonical Amount/ledger tuple, listener singleton, and peer order before
/// this body can run.
fn monologue_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !monologue_program_is_exact(ctx.spec, ctx.catalog)
        || ctx.args != [CompiledArg::I(1)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.state.history.over
        || !monologue_private_state_is_exact(ctx.state)
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
    {
        return Err(EngineRefusal::MalformedArgs("monologue foundation"));
    }
    let (pile, index) = crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        })?;
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "monologue foundation physical source",
        ));
    }

    fn apply(
        state: &mut HotState,
        source_uid: u32,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let prior = state.powers.value(PowerId::Monologue);
        let updated = prior
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("monologue amount"))?;
        let created_uid = state
            .fanouts
            .begin_monologue_instance(1, 1)
            .map_err(|_| EngineRefusal::CounterOverflow("Monologue instance uid"))?;
        crate::engine::play::record_instanced_power_created_uid(state, source_uid, created_uid)?;
        state.powers.set(PowerId::Monologue, SlotWire::Int, updated);
        // PowerCmd/<Apply>d__2 (RVA 0x3efbac) reports the fresh row amount.
        // Match the established instanced The Bomb projection: the uid-less
        // event carries this object's amount, while the cold row carries uid.
        crate::engine::damage::note_power(events, Subject::Player, PowerId::Monologue, 1);
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(&mut probe, ctx.source_uid, &mut Vec::new())?;
    if !monologue_private_state_is_exact(&probe) {
        return Err(EngineRefusal::MalformedArgs("monologue foundation result"));
    }
    *ctx.state = probe;
    crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::Monologue, 1);
    Ok(())
}

/// `("plot_exact", amount)` — exact ordered DrawNextTurn fan-out (#1562).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) → `_apply_draw_next_turn_to_all_allies`, the complete live-player DrawCardsNextTurn application loop.
///
/// The local recipient uses the existing DrawNextTurn power slot; the remote
/// recipient uses its exact scalar quotient. Unsupported application
/// listeners and overflow refuse at admission before the ordered batch.
pub(crate) fn plot_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(amount)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("plot_exact"));
    };
    if ctx.spec.identity.id != CardId::Plot || *amount != 2 + i64::from(ctx.spec.identity.upgrade) {
        return Err(EngineRefusal::MalformedArgs("plot_exact"));
    }
    let amount: i32 = (*amount)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("plot_exact"))?;
    let mut probe = ctx.state.clone();
    for key in crate::engine::allies::living_keys(&probe).collect::<Vec<_>>() {
        crate::engine::allies::add_draw_next_turn(&mut probe, key, amount)?;
    }
    for key in crate::engine::allies::living_keys(ctx.state).collect::<Vec<_>>() {
        crate::engine::allies::add_draw_next_turn(ctx.state, key, amount)?;
    }
    Ok(())
}

/// `("quasar_exact",)` — exact skippable generated-card screen.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) delegates to
/// `_begin_quasar_selection_exact`, which consumes the shuffle before
/// publishing the skippable three-option continuation.
///
/// Current v0.111.0 IL (assembly SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`) pins
/// `Quasar/<OnPlay>d__3::MoveNext` at RVA `0x3b4e48`: after the play pipeline
/// spends its fixed two Stars, `GetDistinctForCombat` (`0x112878`) performs
/// one complete `UnstableShuffle` (`0x1131e4`) of the recorded profile's
/// Colorless pool (50 rows, 49 draws, when fully unlocked; #2560), the upgraded
/// source upgrades all three options, and a null choice mints nothing. A
/// non-null choice alone reaches the singular generated-card command.
pub(crate) fn begin_quasar_exact(
    ctx: &mut StepCtx<'_>,
) -> Result<Vec<crate::hot::HotCard>, EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Quasar
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !crate::engine::cards::colorless_generation_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs("quasar_exact"));
    }
    // #2560: the recorded profile's Colorless pool (`GetUnlockedCards` at
    // `0x3b4e48` IL_004d), `REGENT_COLORLESS_GENERATION_POOL_V1101` when fully
    // unlocked. No derivable profile leaves fewer than three rows, but a
    // shorter pool would be a different screen, so refuse it by name.
    let pool = ctx.catalog.colorless_generation_pool();
    if pool.len() < 3 {
        return Err(EngineRefusal::MalformedArgs("quasar_exact pool"));
    }
    let mut probe = ctx.state.clone();
    let shuffled = shuffle_generation_slice(&mut probe, &pool)?;
    let options = shuffled[..3]
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
            Ok(crate::hot::HotCard {
                uid: 0,
                atom,
                flags: 0,
            })
        })
        .collect::<Result<Vec<_>, EngineRefusal>>()?;
    ctx.state
        .rng
        .set(RngStream::Generation, probe.rng.get(RngStream::Generation));
    Ok(options)
}

pub(crate) fn quasar_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = begin_quasar_exact(ctx)?;
    Err(EngineRefusal::ContinuationNotModeled)
}

/// `("tutor_exact",)` — exact selected-Player Draw-pile fetch.
///
/// Current v0.111.0 authority (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Tutor::.ctor` RVA `0xef46a`, `get_MultiplayerConstraint` RVA `0xef477`,
/// `OnUpgrade` RVA `0xef4cf`, and `Tutor/<OnPlay>d__3::MoveNext` RVA
/// `0x3c4ddc`. The coroutine resolves `cardPlay.Target.Player`, awaits one
/// exact Draw-pile selection, takes the first selected physical card, and
/// adds it to the same Player's Hand at Bottom (native Hand capacity redirects
/// that Add to Discard).
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) builds the local live-pile selector and
/// `_apply_tutor_remote_fetch` commits the choice-free remote quotient.
/// Rust uses the existing stable Player key and remote pile COW carrier. Only
/// the owner-local multi-card screen parks; its answer is the selected
/// physical uid, so duplicate payloads remain distinct without new frame
/// state.
pub(crate) fn begin_tutor_exact(ctx: &mut StepCtx<'_>) -> Result<bool, EngineRefusal> {
    if !ctx.args.is_empty() || ctx.selection.is_some() || ctx.x_value != 0 {
        return Err(EngineRefusal::MalformedArgs("tutor_exact"));
    }
    preflight_tutor_exact(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let key = constellation_target_key(ctx.state, ctx.target)?;
    if key == 1 {
        crate::engine::allies::tutor_remote_fetch(ctx.state, key)?;
        return Ok(false);
    }
    crate::engine::cards::normalize_card_identities(ctx.state)?;
    for (index, card) in ctx
        .state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .enumerate()
    {
        if crate::engine::play::unique_live_card_location(ctx.state, card.uid)?
            != Some((PileId::Draw, index))
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    match ctx.state.piles.get(PileId::Draw).len() {
        0 => Ok(false),
        1 => {
            let uid = ctx.state.piles.get(PileId::Draw).as_slice()[0].uid;
            crate::engine::selection::move_seeker_card(ctx.state, uid, ctx.events)?;
            Ok(false)
        }
        _ => Ok(true),
    }
}

pub(crate) fn tutor_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if begin_tutor_exact(ctx)? {
        Err(EngineRefusal::ContinuationNotModeled)
    } else {
        Ok(())
    }
}

fn tutor_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    matches!(
        (spec.identity.id, spec.identity.upgrade),
        (CardId::Tutor, 0 | 1)
    ) && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) == Some(spec.row)
        && matches!(catalog.steps(spec), [step] if
            step.kind == StepKind::TutorExact && catalog.args(step.args).is_empty())
}

pub(crate) fn preflight_tutor_exact(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    if !tutor_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs("tutor_exact program"));
    }
    exact_constellation_source(state, catalog, source_uid, spec)?;
    if crate::engine::allies::living_keys(state).count() <= 1 {
        return Err(EngineRefusal::MalformedArgs("Tutor MultiplayerOnly"));
    }
    if state.multiplayer_ally_key == 1
        && state.fanouts.multiplayer_ally().alive
        && state.fanouts.multiplayer_ally().draw.len() > 1
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

/// `("void_form", 2)` — exact Void Form power application and deferred
/// native EndTurn request.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). Current v0.111.0
/// `VoidForm/<OnPlay>d__5::MoveNext` RVA `0x3c69e8`
/// awaits the generic Amount-two power apply and then requests EndTurn.  The
/// private carrier below now models its exact amount/counter/hook bundle and
/// the engine services that request only after the root play or resumed
/// continuation has fully unwound.  The corrected Python oracle pins the
/// same current-build sentinel and deferred service.
///
pub(crate) fn void_form(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    void_form_foundation_exact(ctx)
}

fn void_form_program_is_exact(spec: &CardSpec, catalog: &Catalog) -> bool {
    matches!(
        (spec.identity.id, spec.identity.upgrade),
        (CardId::VoidForm, 0 | 1)
    ) && crate::engine::play::body_enchantment_is_exact(spec)
        && crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            == Some(spec.row)
        && matches!(
            spec.row.steps,
            [crate::content_tables::Step {
                kind: StepKind::VoidForm,
                args: [Arg::I(2)]
            }]
        )
        && matches!(
            catalog.steps(spec),
            [step]
                if step.kind == StepKind::VoidForm
                    && catalog.args(step.args) == [CompiledArg::I(2)]
        )
}

/// Authenticate Void Form's deliberately private singleton quotient.
///
/// Current-v0.111 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `VoidFormPower::BeforeApplied` RVA `0xaac8e` and
/// `BeforePowerAmountChanged` RVA `0xaac77` both call
/// `HideTemporaryZeroCostVisual` RVA `0xaae10`, which stores the exact
/// `999_999_999` completed-play sentinel. The compact hook bit represents
/// the complete cost/AfterCardPlayed/BeforeSideTurnStart listener bundle.
pub(crate) fn void_form_private_state_is_exact(state: &HotState) -> bool {
    let amount = match state.powers.get(PowerId::VoidForm) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value > 0 => slot.value,
        Some(_) => return false,
    };
    let completed = state.fanouts.void_form_cards_played_this_turn();
    let hooks = state.fanouts.void_form_hooks_are_registered();
    let requested = state.fanouts.void_form_end_turn_requested();
    (amount == 0 && completed == 0 && !hooks && !requested)
        || (amount > 0
            && completed >= 0
            && hooks
            && (!requested || completed >= 999_999_999)
            && state.multiplayer_ally_key == 0
            && crate::engine::damage::player_type_one_listener_order_is_exact(state))
}

/// Whether an action can observe a live, orphaned, or partially-forged Void
/// Form private state and therefore must authenticate/transaction the seam.
pub(crate) fn void_form_listener_is_reachable(state: &HotState) -> bool {
    state.powers.get(PowerId::VoidForm).is_some()
        || state.fanouts.void_form_cards_played_this_turn() != 0
        || state.fanouts.void_form_hooks_are_registered()
        || state.fanouts.void_form_end_turn_requested()
}

fn void_form_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !void_form_program_is_exact(ctx.spec, ctx.catalog)
        || ctx.args != [CompiledArg::I(2)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.state.history.over
        || !void_form_private_state_is_exact(ctx.state)
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
    {
        return Err(EngineRefusal::MalformedArgs("void form foundation"));
    }
    let (pile, index) = crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        })?;
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "void form foundation physical source",
        ));
    }

    fn apply(state: &mut HotState, events: &mut Vec<Event>) -> Result<(), EngineRefusal> {
        if !crate::engine::damage::player_type_one_listener_order_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs(
                "after-power-amount-changed listener order",
            ));
        }
        let prior = state.powers.value(PowerId::VoidForm);
        let updated = prior
            .checked_add(2)
            .ok_or(EngineRefusal::CounterOverflow("void form amount"))?;
        // Native performs this synchronous private callback before the
        // generic amount write on both first application and restack.
        let written = state
            .fanouts
            .set_void_form_cards_played_this_turn(999_999_999);
        debug_assert!(written);
        crate::engine::play::prepare_after_card_played_scalar_write(
            state,
            PowerId::VoidForm,
            prior,
            updated,
        )?;
        if prior == 0 {
            state.fanouts.set_void_form_hooks_registered(true);
        }
        state.powers.set(PowerId::VoidForm, SlotWire::Int, updated);
        crate::engine::damage::note_power(events, Subject::Player, PowerId::VoidForm, updated);
        state.fanouts.set_void_form_end_turn_requested(true);
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(&mut probe, &mut Vec::new())?;
    if !void_form_private_state_is_exact(&probe) {
        return Err(EngineRefusal::MalformedArgs("void form foundation result"));
    }
    apply(ctx.state, ctx.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS, REGENT_COLORLESS_GENERATION_POOL_V1101};
    use crate::engine::{Action, SelectionAnswer, SelectionRef, apply_action};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CardInstanceState, HotCard, HotMonster, HotState,
        MultiplayerAllyCard, MultiplayerAllyState, PileId, RngStreamState,
    };
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;
    use crate::rng::Xoshiro256StarStar;

    /// The gate Bundle of Joy and Manifest Authority share (#2560), row by
    /// row: any owner, an absent-or-agreeing Entropy owner, a recorded
    /// profile, solo, a live Generation stream, and — for a Regent owner only
    /// — the published `spectrum_shift_generation_pool`.
    #[test]
    fn colorless_body_provenance_truth_table() {
        let catalog = CatalogBuilder::new().build();
        let base = |owner: RewardPool| {
            let mut state = HotState::at_defaults();
            state.reward_card_pool = Some(owner);
            state.entropy_card_pool = None;
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
        };
        type Case = (&'static str, RewardPool, bool, fn(&mut HotState));
        let cases: [Case; 9] = [
            ("Regent, published pool", RewardPool::Regent, true, |_| {}),
            (
                "Defect, absent Entropy owner",
                RewardPool::Defect,
                true,
                |_| {},
            ),
            (
                "Silent, agreeing Entropy owner",
                RewardPool::Silent,
                true,
                |s| {
                    s.entropy_card_pool = Some(RewardPool::Silent);
                },
            ),
            (
                "contradictory Entropy owner",
                RewardPool::Defect,
                false,
                |s| {
                    s.entropy_card_pool = Some(RewardPool::Ironclad);
                },
            ),
            ("Regent, unpublished pool", RewardPool::Regent, false, |s| {
                s.set_spectrum_shift_generation_pool(false);
            }),
            ("unrecorded profile", RewardPool::Defect, false, |s| {
                s.fully_unlocked_card_pool_epochs = false;
            }),
            ("party", RewardPool::Defect, false, |s| {
                s.multiplayer_ally_key = 1
            }),
            ("no Generation stream", RewardPool::Defect, false, |s| {
                s.rng.set(RngStream::Generation, RngStreamState::default());
            }),
            ("no owner", RewardPool::Defect, false, |s| {
                s.reward_card_pool = None
            }),
        ];
        for (label, owner, expected, mutate) in cases {
            let mut state = base(owner);
            mutate(&mut state);
            assert_eq!(
                colorless_body_provenance_is_exact(&state, &catalog),
                expected,
                "{label}"
            );
        }
    }

    /// This file's own kind inventory — the eleven stubs below, in the order
    /// the generator wrote them.
    ///
    /// Deliberately a local list rather than a filter over
    /// `crate::steps::FAMILY_OF`: selecting this family out of that table
    /// means comparing its name, and `tests/hot_path_contract.rs` bans text
    /// comparison anywhere in the dispatch tree (D2). The cross-check that the
    /// table and this list agree is
    /// [`the_family_claims_nothing_yet_and_never_claims_a_foreign_kind`],
    /// which does it through `is_implemented` instead of through a name.
    const OWNED: [StepKind; 11] = [
        StepKind::BundleOfJoyExact,
        StepKind::ConstellationExact,
        StepKind::GuidingStarDrawNextTurnExact,
        StepKind::HuddleUpExact,
        StepKind::LargesseExact,
        StepKind::ManifestAuthorityExact,
        StepKind::Monologue,
        StepKind::PlotExact,
        StepKind::QuasarExact,
        StepKind::TutorExact,
        StepKind::VoidForm,
    ];

    fn fixture() -> (HotState, Catalog) {
        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/canonical_state_v2_ironclad_toadpoles.json"
        ))
        .unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (state, catalog)
    }

    fn monologue_parts(upgrade: u8) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Monologue,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 41,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 42;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source)
    }

    fn run_monologue_foundation(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::play::with_test_active_play(source.uid, || {
            monologue_foundation_exact(&mut StepCtx {
                state,
                catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(1)],
                events,
            })
        })
    }

    #[test]
    fn monologue_writer_stacks_exactly_and_public_dispatch_is_exact() {
        for upgrade in [0, 1] {
            let (mut state, catalog, source) = monologue_parts(upgrade);
            let mut events = Vec::new();
            run_monologue_foundation(&mut state, &catalog, source, &mut events).unwrap();
            run_monologue_foundation(&mut state, &catalog, source, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::Monologue), 2);
            assert_eq!(state.fanouts.monologue_strength_applied(), 0);
            assert!(state.fanouts.monologue_hooks_are_registered());
            assert!(monologue_private_state_is_exact(&state));
            assert_eq!(events.len(), 2);
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        Event::PowerChanged {
                            power: PowerId::Monologue,
                            amount,
                            ..
                        } => Some(*amount),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                [1, 1],
                "PowerReceived projects each fresh native object, never aggregate Amount"
            );

            let spec = *catalog.spec(source.atom).unwrap();
            let mut public_events = Vec::new();
            crate::engine::play::with_test_active_play(source.uid, || {
                monologue(&mut StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(1)],
                    events: &mut public_events,
                })
            })
            .unwrap();
            assert_eq!(state.powers.value(PowerId::Monologue), 3);
            assert_eq!(public_events.len(), 1);
            assert!(IMPLEMENTED.contains(&StepKind::Monologue));
            assert!(
                crate::engine::admission::IMPLEMENTED_PLAYER_POWERS.contains(&PowerId::Monologue)
            );
        }
    }

    #[test]
    fn monologue_public_actions_cover_first_cast_recast_and_late_rollback() {
        for upgrade in [0, 1] {
            let (mut state, catalog, first) = monologue_parts(upgrade);
            state.exact_piles = false;
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let second = HotCard { uid: 42, ..first };
            state.next_card_uid = 43;
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .extend([first, second]);

            let first_result = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: first.uid,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert_eq!(first_result.powers.value(PowerId::Monologue), 1);
            assert_eq!(first_result.powers.value(PowerId::Strength), 0);
            assert_eq!(first_result.fanouts.monologue_strength_applied(), 0);

            let recast = apply_action(
                &first_result,
                &catalog,
                &Action::Play {
                    uid: second.uid,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert_eq!(recast.powers.value(PowerId::Monologue), 2);
            assert_eq!(recast.powers.value(PowerId::Strength), 1);
            assert_eq!(recast.fanouts.monologue_strength_applied(), 1);
            assert_eq!(
                recast
                    .fanouts
                    .monologue_instances()
                    .map(|row| (row.uid, row.power, row.strength_applied))
                    .collect::<Vec<_>>(),
                [(0, 1, 1), (1, 1, 0)],
                "the older object consumes its BeforeCardPlayed latch; the new object has none"
            );
            assert_eq!(
                recast.piles.get(PileId::Discard).as_slice(),
                &[first, second]
            );

            let mut overflow = first_result;
            let uid = overflow.fanouts.monologue_instances().next().unwrap().uid;
            assert!(
                overflow
                    .fanouts
                    .set_monologue_instances(&[crate::hot::MonologueInstance {
                        uid,
                        amount: 1,
                        power: 1,
                        strength_applied: i32::MAX,
                    }])
            );
            let before = overflow.clone();
            assert_eq!(
                apply_action(
                    &overflow,
                    &catalog,
                    &Action::Play {
                        uid: second.uid,
                        target: None,
                        selection: SelectionRef::NONE,
                    },
                ),
                Err(EngineRefusal::CounterOverflow("monologue strength applied"))
            );
            assert_eq!(
                overflow, before,
                "the public action input remains immutable"
            );
        }
    }

    #[test]
    fn monologue_generated_row_source_census_is_exact() {
        let sources: Vec<_> = crate::content_tables::CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::Monologue)
            })
            .map(|row| (row.id, row.upgrade))
            .collect();
        assert_eq!(
            sources,
            [(CardId::Monologue, 0), (CardId::Monologue, 1)],
            "only the two Monologue rows carry the generated Monologue step"
        );
    }

    #[test]
    fn monologue_private_writer_overflow_and_forgery_are_atomic() {
        let (mut overflow, catalog, source) = monologue_parts(0);
        overflow
            .fanouts
            .set_next_after_side_turn_end_power_uid(u32::MAX);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_monologue_foundation(&mut overflow, &catalog, source, &mut events),
            Err(EngineRefusal::CounterOverflow("Monologue instance uid"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let (mut forged, catalog, source) = monologue_parts(0);
        forged.powers.set(PowerId::Monologue, SlotWire::Bool, 1);
        let before = forged.clone();
        assert_eq!(
            run_monologue_foundation(&mut forged, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("monologue foundation"))
        );
        assert_eq!(forged, before);

        let (mut partial, catalog, source) = monologue_parts(0);
        partial.fanouts.forge_monologue_hook_flags_for_test(0b011);
        let before = partial.clone();
        assert_eq!(
            run_monologue_foundation(&mut partial, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("monologue foundation"))
        );
        assert_eq!(partial, before);
    }

    fn void_form_parts(upgrade: u8) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let mut selected = None;
        for level in [0, 1] {
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::VoidForm,
                    upgrade: level,
                    enchantment: None,
                })
                .unwrap();
            if level == upgrade {
                selected = Some(atom);
            }
        }
        let catalog = builder.build();
        let source = HotCard {
            uid: 51,
            atom: selected.unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.exact_piles = true;
        state.next_card_uid = 52;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source)
    }

    fn run_void_form_foundation(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::play::with_test_active_play(source.uid, || {
            void_form_foundation_exact(&mut StepCtx {
                state,
                catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(2)],
                events,
            })
        })
    }

    #[test]
    fn void_form_writer_sets_each_native_sentinel_and_public_dispatch_is_exact() {
        for upgrade in [0, 1] {
            let (mut state, catalog, source) = void_form_parts(upgrade);
            let mut events = Vec::new();
            run_void_form_foundation(&mut state, &catalog, source, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::VoidForm), 2);
            assert_eq!(
                state.fanouts.void_form_cards_played_this_turn(),
                999_999_999
            );
            assert!(state.fanouts.void_form_hooks_are_registered());
            assert!(state.fanouts.void_form_end_turn_requested());
            assert!(void_form_private_state_is_exact(&state));

            assert!(
                state
                    .fanouts
                    .set_void_form_cards_played_this_turn(1_000_000_000)
            );
            run_void_form_foundation(&mut state, &catalog, source, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::VoidForm), 4);
            assert_eq!(
                state.fanouts.void_form_cards_played_this_turn(),
                999_999_999
            );
            assert_eq!(events.len(), 2);

            let spec = *catalog.spec(source.atom).unwrap();
            let mut public_events = Vec::new();
            crate::engine::play::with_test_active_play(source.uid, || {
                void_form(&mut StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(2)],
                    events: &mut public_events,
                })
            })
            .unwrap();
            assert_eq!(state.powers.value(PowerId::VoidForm), 6);
            assert_eq!(public_events.len(), 1);
            assert!(IMPLEMENTED.contains(&StepKind::VoidForm));
            assert!(
                crate::engine::admission::IMPLEMENTED_PLAYER_POWERS.contains(&PowerId::VoidForm)
            );
        }
    }

    #[test]
    fn void_form_private_writer_source_census_overflow_and_forgery_are_exact() {
        let sources: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| step.kind == StepKind::VoidForm))
            .map(|row| (row.id, row.upgrade))
            .collect();
        assert_eq!(sources, [(CardId::VoidForm, 0), (CardId::VoidForm, 1)]);

        let (mut overflow, catalog, source) = void_form_parts(0);
        overflow
            .powers
            .set(PowerId::VoidForm, SlotWire::Int, i32::MAX - 1);
        overflow.fanouts.set_void_form_hooks_registered(true);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_void_form_foundation(&mut overflow, &catalog, source, &mut events),
            Err(EngineRefusal::CounterOverflow("void form amount"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let (mut wrong_wire, catalog, source) = void_form_parts(0);
        wrong_wire.powers.set(PowerId::VoidForm, SlotWire::Bool, 1);
        let before = wrong_wire.clone();
        assert_eq!(
            run_void_form_foundation(&mut wrong_wire, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("void form foundation"))
        );
        assert_eq!(wrong_wire, before);

        let (mut orphan, catalog, source) = void_form_parts(0);
        orphan.fanouts.set_void_form_hooks_registered(true);
        let before = orphan.clone();
        assert_eq!(
            run_void_form_foundation(&mut orphan, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("void form foundation"))
        );
        assert_eq!(orphan, before);

        let (mut low_request, catalog, source) = void_form_parts(0);
        low_request.powers.set(PowerId::VoidForm, SlotWire::Int, 2);
        low_request.fanouts.set_void_form_hooks_registered(true);
        assert!(low_request.fanouts.set_void_form_cards_played_this_turn(7));
        low_request.fanouts.set_void_form_end_turn_requested(true);
        let before = low_request.clone();
        assert_eq!(
            run_void_form_foundation(&mut low_request, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("void form foundation"))
        );
        assert_eq!(low_request, before);
    }

    #[test]
    fn void_form_private_writer_requires_current_physical_source_and_exact_type_one_order() {
        let (mut no_context, catalog, source) = void_form_parts(0);
        let spec = *catalog.spec(source.atom).unwrap();
        let before = no_context.clone();
        assert_eq!(
            void_form_foundation_exact(&mut StepCtx {
                state: &mut no_context,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(2)],
                events: &mut Vec::new(),
            }),
            Err(EngineRefusal::MalformedArgs("void form foundation"))
        );
        assert_eq!(no_context, before);

        let (mut suspended_parent, catalog, source) = void_form_parts(0);
        let before = suspended_parent.clone();
        let result = crate::engine::play::with_test_active_play(source.uid, || {
            crate::engine::play::with_test_active_play(source.uid + 1, || {
                let spec = *catalog.spec(source.atom).unwrap();
                void_form_foundation_exact(&mut StepCtx {
                    state: &mut suspended_parent,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target: None,
                    selection: None,
                    x_value: 0,
                    args: &[CompiledArg::I(2)],
                    events: &mut Vec::new(),
                })
            })
        });
        assert_eq!(
            result,
            Err(EngineRefusal::MalformedArgs("void form foundation"))
        );
        assert_eq!(suspended_parent, before);

        let (mut duplicate, catalog, source) = void_form_parts(0);
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        assert_eq!(
            run_void_form_foundation(&mut duplicate, &catalog, source, &mut Vec::new()),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(duplicate, before);

        let (base, catalog, source) = void_form_parts(0);
        let mut forgeries = Vec::new();
        for power in [
            PowerId::Vicious,
            PowerId::Shroud,
            PowerId::SleightOfFlesh,
            PowerId::SwordSage,
        ] {
            let mut amount_without_listener = base.clone();
            amount_without_listener.powers.set(power, SlotWire::Int, 1);
            forgeries.push(amount_without_listener);

            let mut listener_without_amount = base.clone();
            assert!(
                listener_without_amount
                    .fanouts
                    .set_after_power_amount_changed_order(&[power])
            );
            forgeries.push(listener_without_amount);

            let mut wrong_wire = base.clone();
            wrong_wire.powers.set(power, SlotWire::Bool, 1);
            assert!(
                wrong_wire
                    .fanouts
                    .set_after_power_amount_changed_order(&[power])
            );
            forgeries.push(wrong_wire);
        }
        let mut duplicate_listener = base.clone();
        duplicate_listener
            .powers
            .set(PowerId::Vicious, SlotWire::Int, 1);
        assert!(
            duplicate_listener
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious, PowerId::Vicious])
        );
        forgeries.push(duplicate_listener);
        for mut forged in forgeries {
            let before = forged.clone();
            let mut events = vec![Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::Strength,
                amount: 77,
            }];
            let before_events = events.clone();
            assert_eq!(
                run_void_form_foundation(&mut forged, &catalog, source, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                ))
            );
            assert_eq!(forged, before);
            assert_eq!(events, before_events);
        }
    }

    fn guiding_star_parts(upgrade: u8, monster_hp: i32) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::GuidingStar,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 1;
        state.stars = 1;
        state.next_card_uid = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, monster_hp));
        let source = HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        (state, catalog, source)
    }

    fn constellation_parts(upgrade: u8) -> (HotState, Catalog, HotCard, HotCard) {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::Constellation,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let draw_atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeRegent,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        // GainEnergy checks IsEnding, which is trivially true for a
        // monsterless state; Constellation is played in live combat.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.energy = 3;
        state.block = 4;
        state.next_card_uid = 3;
        let source = HotCard {
            uid: 1,
            atom: source_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let drawn = HotCard {
            uid: 2,
            atom: draw_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.piles.get_mut(PileId::Draw).make_mut().push(drawn);
        (state, catalog, source, drawn)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_constellation(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        target: Option<usize>,
        selection: Option<u32>,
        x_value: i64,
        args: &[CompiledArg],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        constellation_exact(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target,
            selection,
            x_value,
            args,
            events,
        })
    }

    #[test]
    fn constellation_current_build_rows_and_program_are_exactly_both_levels() {
        let rows = CARD_ROWS
            .iter()
            .filter(|row| matches!(row.id, CardId::Constellation))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        for (row, name, block) in [
            (rows[0], "CONSTELLATION", 9),
            (rows[1], "CONSTELLATION+", 12),
        ] {
            assert_eq!(row.name, name);
            assert_eq!((row.cost, row.star_cost), (0, 2));
            assert!(row.playable && row.is_skill && row.targeted);
            assert_eq!(row.target_type, "AnyAlly");
            assert!(!row.is_power && !row.exhausts && !row.selects);
            assert_eq!(row.steps.len(), 1);
            assert_eq!(row.steps[0].kind, StepKind::ConstellationExact);
            assert_eq!(row.steps[0].args, &[Arg::I(1), Arg::I(1), Arg::I(block)]);

            let (_, catalog, source, _) = constellation_parts(u8::from(row.name.ends_with('+')));
            let spec = catalog.spec(source.atom).unwrap();
            assert_eq!(
                constellation_program(&catalog, spec),
                Some((1, 1, i32::try_from(block).unwrap()))
            );
        }
        assert!(
            crate::content_tables::GENERATION_POOLS
                .iter()
                .all(|(_, pool)| !pool.contains(&CardId::Constellation)),
            "no current generated-card pool can mint the MultiplayerOnly row"
        );
    }

    #[test]
    fn constellation_selected_owner_draws_then_gains_energy_then_powered_block() {
        for (upgrade, block) in [(0, 9), (1, 12)] {
            let (mut state, catalog, source, drawn) = constellation_parts(upgrade);
            let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(block)];
            let mut events = Vec::new();

            run_constellation(
                &mut state,
                &catalog,
                source,
                Some(0),
                None,
                0,
                &args,
                &mut events,
            )
            .unwrap();

            assert_eq!(state.piles.get(PileId::Hand).as_slice(), &[drawn]);
            assert!(state.piles.get(PileId::Draw).as_slice().is_empty());
            assert_eq!(state.energy, 4);
            assert_eq!(state.block, 4 + block as i32);
            assert_eq!(
                events,
                vec![
                    Event::CardDrawn { uid: drawn.uid },
                    Event::PlayerBlockGained {
                        amount: block as i32,
                        block: 4 + block as i32,
                    },
                ]
            );
        }
    }

    #[test]
    fn constellation_remote_target_owns_its_piles_resources_and_no_energy_power() {
        let (mut state, catalog, source, _) = constellation_parts(1);
        let remote_draw = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            draw: std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(remote_draw).unwrap()]),
            energy: 7,
            block: 8,
            ..MultiplayerAllyState::default()
        });
        let local_before = (
            state.piles.get(PileId::Draw).as_slice().to_vec(),
            state.energy,
            state.block,
        );
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(12)];
        let mut events = Vec::new();

        run_constellation(
            &mut state,
            &catalog,
            source,
            Some(1),
            None,
            0,
            &args,
            &mut events,
        )
        .unwrap();

        let ally = state.fanouts.multiplayer_ally();
        assert!(ally.draw.is_empty());
        assert_eq!(ally.hand[0].identity, remote_draw);
        assert_eq!((ally.energy, ally.block), (8, 20));
        assert_eq!(
            (
                state.piles.get(PileId::Draw).as_slice().to_vec(),
                state.energy,
                state.block,
            ),
            local_before
        );
        assert!(
            events.is_empty(),
            "remote commands do not publish owner events"
        );

        let mut capped = state.clone();
        {
            let ally = capped.fanouts.multiplayer_ally_mut();
            ally.draw =
                std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(remote_draw).unwrap()]);
            ally.hand = std::sync::Arc::new(vec![
                MultiplayerAllyCard::immutable(remote_draw)
                    .unwrap();
                10
            ]);
            ally.no_energy_gain = true;
        }
        let before_energy = capped.fanouts.multiplayer_ally().energy;
        run_constellation(
            &mut capped,
            &catalog,
            source,
            Some(1),
            None,
            0,
            &args,
            &mut Vec::new(),
        )
        .unwrap();
        let ally = capped.fanouts.multiplayer_ally();
        assert_eq!(ally.draw[0].identity, remote_draw);
        assert_eq!(ally.hand.len(), 10);
        assert_eq!(ally.energy, before_energy);
        assert_eq!(ally.block, 32, "NoEnergyGain does not suppress Block");
    }

    /// #2708: Constellation's Energy is `PlayerCmd::GainEnergy`
    /// (`Constellation/<OnPlay>d__11` 0x394148 IL_01e9), which returns at
    /// IsEnding (0x3ee8a0 IL_0035). With the only primary dead before the
    /// over latch the tail's Energy skips.
    #[test]
    fn constellation_energy_skips_while_combat_is_ending_before_the_over_latch() {
        let (mut ending, catalog, source, _) = constellation_parts(0);
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(9)];
        run_constellation(
            &mut ending,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(ending.energy, 3);
    }

    /// #3112: Constellation's Draw (`CardPileCmd::Draw` IL_0170, returning at
    /// IsOverOrEnding in `<DrawInternal>d__21` 0x3e3a70 IL_002e) and Block
    /// (`CreatureCmd::GainBlock` IL_0263, returning at IsOverOrEnding in
    /// `<GainBlock>d__18` 0x3eaec0 IL_0032) are both skipped while combat is
    /// ending before history.over latches. The whole play draws nothing, and
    /// the post-Draw tail (the resumed-continuation entry) gains neither
    /// Energy nor Block. The live control draws and gains both.
    #[test]
    fn constellation_draw_and_block_skip_while_combat_is_ending_before_the_over_latch() {
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(9)];
        let (mut ending, catalog, source, drawn) = constellation_parts(0);
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let mut events = Vec::new();
        run_constellation(
            &mut ending,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut events,
        )
        .unwrap();
        assert_eq!(ending.piles.get(PileId::Draw).as_slice(), &[drawn]);
        assert!(ending.piles.get(PileId::Hand).as_slice().is_empty());
        assert_eq!((ending.energy, ending.block), (3, 4));
        assert!(events.is_empty());

        let spec = *catalog.spec(source.atom).unwrap();
        let (mut tail, _, _, _) = constellation_parts(0);
        tail.monsters_mut()[0].hp = 0;
        let mut events = Vec::new();
        resume_constellation_after_draw(&mut tail, &catalog, &spec, Some(0), &mut events).unwrap();
        assert_eq!((tail.energy, tail.block), (3, 4));
        assert!(events.is_empty());

        let (mut live, catalog, source, drawn) = constellation_parts(0);
        run_constellation(
            &mut live,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(live.piles.get(PileId::Hand).as_slice(), &[drawn]);
        assert_eq!((live.energy, live.block), (4, 13));
        let (mut live_tail, _, _, _) = constellation_parts(0);
        resume_constellation_after_draw(&mut live_tail, &catalog, &spec, Some(0), &mut Vec::new())
            .unwrap();
        assert_eq!((live_tail.energy, live_tail.block), (4, 13));
    }

    #[test]
    fn constellation_no_draw_suppresses_only_draw_and_terminal_draw_suppresses_the_tail() {
        let (mut no_draw, catalog, source, drawn) = constellation_parts(0);
        no_draw.powers.set(PowerId::NoDraw, SlotWire::Int, 1);
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(9)];
        let mut events = Vec::new();
        run_constellation(
            &mut no_draw,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut events,
        )
        .unwrap();
        assert_eq!(no_draw.piles.get(PileId::Draw).as_slice(), &[drawn]);
        assert_eq!((no_draw.energy, no_draw.block), (4, 13));
        assert_eq!(
            events,
            vec![Event::PlayerBlockGained {
                amount: 9,
                block: 13,
            }]
        );

        let (mut terminal, catalog, source, _) = constellation_parts(0);
        *terminal.monsters_mut() = vec![HotMonster::new(MonsterKind::Toadpole, 1)];
        terminal.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        assert!(
            terminal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        terminal.fanouts.set_cacophony_left(1);
        let seeded = Xoshiro256StarStar::from_seed(41);
        terminal.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        let mut events = Vec::new();
        run_constellation(
            &mut terminal,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut events,
        )
        .unwrap();
        assert!(terminal.history.over);
        assert_eq!((terminal.energy, terminal.block), (3, 4));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Event::PlayerBlockGained { .. }))
        );
    }

    #[test]
    fn constellation_late_refusals_and_action_shape_are_whole_body_atomic() {
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(9)];
        for (target, selection, x_value, expected) in [
            (
                None,
                None,
                0,
                EngineRefusal::TargetMismatch { required: true },
            ),
            (
                Some(0),
                Some(0),
                0,
                EngineRefusal::MalformedArgs("constellation_exact action"),
            ),
            (
                Some(0),
                None,
                1,
                EngineRefusal::MalformedArgs("constellation_exact action"),
            ),
        ] {
            let (mut state, catalog, source, _) = constellation_parts(0);
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                run_constellation(
                    &mut state,
                    &catalog,
                    source,
                    target,
                    selection,
                    x_value,
                    &args,
                    &mut events,
                ),
                Err(expected)
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let (mut overflow, catalog, source, _) = constellation_parts(0);
        overflow.energy = i16::MAX;
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_constellation(
                &mut overflow,
                &catalog,
                source,
                Some(0),
                None,
                0,
                &args,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(overflow, before, "the rehearsed Draw must roll back");
        assert!(events.is_empty());

        let (mut duplicate, catalog, source, _) = constellation_parts(0);
        duplicate
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        assert_eq!(
            run_constellation(
                &mut duplicate,
                &catalog,
                source,
                Some(0),
                None,
                0,
                &args,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(duplicate, before);
    }

    #[test]
    fn constellation_requires_one_physical_source_but_accepts_direct_draw_placement() {
        let args = [CompiledArg::I(1), CompiledArg::I(1), CompiledArg::I(9)];
        let (mut catalog_only, catalog, source, _) = constellation_parts(0);
        catalog_only.piles.get_mut(PileId::Play).make_mut().clear();
        let before = catalog_only.clone();
        assert_eq!(
            run_constellation(
                &mut catalog_only,
                &catalog,
                source,
                Some(0),
                None,
                0,
                &args,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 0,
            })
        );
        assert_eq!(catalog_only, before);

        let (mut direct_draw, catalog, source, drawn) = constellation_parts(0);
        direct_draw.piles.get_mut(PileId::Play).make_mut().clear();
        direct_draw
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .insert(0, source);
        run_constellation(
            &mut direct_draw,
            &catalog,
            source,
            Some(0),
            None,
            0,
            &args,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(direct_draw.piles.get(PileId::Hand).as_slice(), &[source]);
        assert_eq!(direct_draw.piles.get(PileId::Draw).as_slice(), &[drawn]);
    }

    #[test]
    fn constellation_public_manual_spends_fixed_stars_and_rolls_back_a_late_overflow() {
        let (mut ordinary, catalog, source, drawn) = constellation_parts(0);
        ordinary.multiplayer_ally_key = 1;
        ordinary.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        ordinary.stars = 2;
        ordinary.piles.get_mut(PileId::Play).make_mut().clear();
        ordinary.piles.get_mut(PileId::Hand).make_mut().push(source);
        let mut events = Vec::new();
        crate::engine::play::play_card(
            &mut ordinary,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();
        assert_eq!(
            (ordinary.energy, ordinary.stars, ordinary.block),
            (4, 0, 13)
        );
        assert_eq!(ordinary.piles.get(PileId::Hand).as_slice(), &[drawn]);
        assert_eq!(ordinary.piles.get(PileId::Discard).as_slice(), &[source]);

        let (mut overflow, catalog, source, _) = constellation_parts(0);
        overflow.multiplayer_ally_key = 1;
        overflow.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        overflow.stars = 2;
        overflow.energy = i16::MAX;
        overflow.piles.get_mut(PileId::Play).make_mut().clear();
        overflow.piles.get_mut(PileId::Hand).make_mut().push(source);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::play::play_card(
                &mut overflow,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(
            overflow, before,
            "Stars, source, Draw, and history roll back"
        );
        assert!(events.is_empty());
    }

    #[test]
    fn constellation_local_hellraiser_draw_roundtrips_then_runs_tail_once() {
        let constellation = CardIdentity {
            id: CardId::Constellation,
            upgrade: 0,
            enchantment: None,
        };
        let seeker = CardIdentity {
            id: CardId::SeekerStrike,
            upgrade: 0,
            enchantment: None,
        };
        let defend = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        for identity in [constellation, seeker, defend] {
            builder.intern_reachable(identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_hellraiser_reachable();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.block = 4;
        state.stars = 2;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Sel,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&constellation).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: catalog.atom(&seeker).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.next_card_uid = 5;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        state.monsters_mut().push(monster);

        let start = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(crate::engine::admit(&start, &state, &catalog), Ok(()));
        assert_eq!(
            preflight_constellation_exact(
                &state,
                &catalog,
                1,
                catalog.spec(catalog.atom(&constellation).unwrap()).unwrap(),
                Some(0),
            ),
            Ok(())
        );
        let parked = apply_action(
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
        assert_eq!((parked.energy, parked.stars, parked.block), (3, 0, 4));
        assert_eq!(parked.pending.as_deref().unwrap().frame_uid, 2);
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let mut wrong_requested = wire.clone();
        wrong_requested.continuations[2]
            .fields
            .insert("requested".to_owned(), serde_json::json!(2));
        let wrong_catalog = HotBoundary::catalog_from_canonical(&wrong_requested).unwrap();
        assert!(HotBoundary::from_canonical(&wrong_requested, &wrong_catalog).is_err());
        for forged_choice in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!(2),
        ] {
            let mut forged = wire.clone();
            forged.continuations[1]
                .fields
                .insert("choice".to_owned(), forged_choice);
            let forged_catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
            assert!(HotBoundary::from_canonical(&forged, &forged_catalog).is_err());
        }
        let mut forged_identity = wire.clone();
        let parked_source = forged_identity
            .piles
            .get_mut("play")
            .unwrap()
            .iter_mut()
            .find(|card| card.uid == Some(1))
            .unwrap();
        parked_source.id = CardId::DefendRegent.as_str().to_owned();
        let forged_catalog = HotBoundary::catalog_from_canonical(&forged_identity).unwrap();
        assert!(HotBoundary::from_canonical(&forged_identity, &forged_catalog).is_err());
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        crate::engine::admit(&wire, &rebuilt, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            wire
        );

        let answer = crate::engine::legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let completed = apply_action(&rebuilt, &rebuilt_catalog, &answer)
            .unwrap()
            .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(
            (completed.energy, completed.stars, completed.block),
            (4, 0, 13)
        );
        assert_eq!(completed.history.card_plays_finished_combat, 2);

        let mut lethal = state.clone();
        lethal.hp = 1;
        lethal.block = 0;
        lethal.energy = i16::MAX;
        lethal.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);
        let lethal = apply_action(
            &lethal,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(
            lethal.history.over && lethal.pending.is_none() && lethal.frames.is_empty(),
            "over={} hp={} pending={:?} frames={:?}",
            lethal.history.over,
            lethal.hp,
            lethal.pending,
            lethal.frames.as_slice()
        );
        assert_eq!(
            (lethal.energy, lethal.stars, lethal.block),
            (i16::MAX, 0, 0),
            "lethal Hellraiser Thorns commits the child and suppresses Constellation's tail"
        );
        let mut nonlethal_overflow = state.clone();
        nonlethal_overflow.energy = i16::MAX;
        let nonlethal_before = nonlethal_overflow.clone();
        let mut refusal_events = Vec::new();
        assert_eq!(
            crate::engine::apply_action_into(
                &nonlethal_overflow,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
                &mut refusal_events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(nonlethal_overflow, nonlethal_before);
        assert!(refusal_events.is_empty());
        assert_eq!(
            HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
            start,
            "the public action leaves its immutable predecessor untouched"
        );
    }

    #[test]
    fn huddle_local_hellraiser_draw_roundtrips_then_runs_remote_suffix_once() {
        let huddle = CardIdentity {
            id: CardId::HuddleUp,
            upgrade: 0,
            enchantment: None,
        };
        let seeker = CardIdentity {
            id: CardId::SeekerStrike,
            upgrade: 0,
            enchantment: None,
        };
        let defend = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        for identity in [huddle, seeker, defend] {
            builder.intern_reachable(identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_hellraiser_reachable();
        let catalog = builder.build();
        let remote_card = MultiplayerAllyCard::immutable(defend).unwrap();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            draw: std::sync::Arc::new(vec![remote_card, remote_card]),
            ..MultiplayerAllyState::default()
        });
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Sel,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&huddle).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: catalog.atom(&seeker).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: catalog.atom(&defend).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.next_card_uid = 5;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        state.monsters_mut().push(monster);

        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(parked.pending.as_deref().unwrap().frame_uid, 2);
        assert!(parked.fanouts.multiplayer_ally().hand.is_empty());
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let mut wrong_requested = wire.clone();
        wrong_requested.continuations[2]
            .fields
            .insert("requested".to_owned(), serde_json::json!(3));
        let wrong_catalog = HotBoundary::catalog_from_canonical(&wrong_requested).unwrap();
        assert!(HotBoundary::from_canonical(&wrong_requested, &wrong_catalog).is_err());
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        crate::engine::admit(&wire, &rebuilt, &rebuilt_catalog).unwrap();
        let mut changed_roster_wire = wire.clone();
        changed_roster_wire
            .player
            .get_mut("multiplayer_allies")
            .unwrap()[0]["fields"]["alive"] = serde_json::json!(false);
        let changed_catalog = HotBoundary::catalog_from_canonical(&changed_roster_wire).unwrap();
        assert!(HotBoundary::from_canonical(&changed_roster_wire, &changed_catalog).is_err());
        let answer = crate::engine::legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let completed = apply_action(&rebuilt, &rebuilt_catalog, &answer)
            .unwrap()
            .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.multiplayer_ally().hand.len(), 2);
        assert!(completed.fanouts.multiplayer_ally().draw.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 2);

        let mut lethal = state.clone();
        lethal.hp = 1;
        lethal.block = 0;
        lethal.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 2);
        lethal.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            discard: std::sync::Arc::new(vec![remote_card]),
            shuffle_rng: None,
            ..MultiplayerAllyState::default()
        });
        let lethal_completed = apply_action(
            &lethal,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(
            lethal_completed.history.over
                && lethal_completed.pending.is_none()
                && lethal_completed.frames.is_empty()
        );
        assert!(lethal_completed.fanouts.multiplayer_ally().hand.is_empty());

        let mut nonlethal = lethal.clone();
        nonlethal.hp = 50;
        nonlethal.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 0);
        let before = nonlethal.clone();
        let mut refusal_events = Vec::new();
        assert_eq!(
            crate::engine::apply_action_into(
                &nonlethal,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                &mut refusal_events,
            ),
            Err(EngineRefusal::MalformedArgs("remote Draw reshuffle stream"))
        );
        assert_eq!(nonlethal, before);
        assert!(refusal_events.is_empty());
    }

    #[test]
    fn constellation_direct_autoplay_rolls_back_target_rng_and_remote_shuffle_refusal() {
        let (mut state, catalog, source, _) = constellation_parts(1);
        state.energy = 0;
        state.stars = 0;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            discard: std::sync::Arc::new(vec![
                MultiplayerAllyCard::immutable(CardIdentity {
                    id: CardId::DefendRegent,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap(),
            ]),
            shuffle_rng: None,
            ..MultiplayerAllyState::default()
        });
        let seeded = Xoshiro256StarStar::from_seed(73);
        state.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::play::autoplay_imitation_clone(
                &mut state,
                &catalog,
                source,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("remote Draw reshuffle stream"))
        );
        assert_eq!(
            state, before,
            "target RNG and the whole child play roll back"
        );
        assert!(events.is_empty());
    }

    #[test]
    fn guiding_star_manifest_and_private_rows_are_exactly_both_levels() {
        assert!(IMPLEMENTED.contains(&StepKind::GuidingStarDrawNextTurnExact));
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::GuidingStarDrawNextTurnExact)
        );
        let rows = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::GuidingStarDrawNextTurnExact)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id, row.upgrade))
                .collect::<Vec<_>>(),
            vec![(CardId::GuidingStar, 0), (CardId::GuidingStar, 1)]
        );
        for (row, name, damage, draw) in [
            (rows[0], "GUIDING_STAR", 12, 2),
            (rows[1], "GUIDING_STAR+", 13, 3),
        ] {
            assert_eq!(row.name, name);
            assert_eq!((row.cost, row.star_cost), (1, 1));
            assert!(row.playable && row.targeted);
            assert_eq!(row.target_type, "AnyEnemy");
            assert!(!row.is_power && !row.is_skill && !row.exhausts);
            assert_eq!(row.steps.len(), 2);
            assert_eq!(row.steps[0].kind, StepKind::Attack);
            assert_eq!(row.steps[0].args, &[Arg::I(damage), Arg::I(1)]);
            assert_eq!(row.steps[1].args, &[Arg::I(draw)]);
        }
    }

    #[test]
    fn guiding_star_manual_play_attacks_spends_then_stacks_delayed_draw() {
        for (upgrade, damage, draw) in [(0, 12, 2), (1, 13, 3)] {
            let (mut state, catalog, source) = guiding_star_parts(upgrade, 100);
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            let before = state.clone();

            let transition = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap();
            let next = transition.state;

            assert_eq!(next.energy, 0, "upgrade {upgrade}");
            assert_eq!(next.stars, 0, "upgrade {upgrade}");
            assert_eq!(next.monsters[0].hp, 100 - damage, "upgrade {upgrade}");
            assert_eq!(
                next.powers.value(PowerId::DrawNextTurn),
                draw,
                "upgrade {upgrade}"
            );
            assert!(next.piles.get(PileId::Hand).as_slice().is_empty());
            assert_eq!(next.piles.get(PileId::Discard).as_slice(), &[source]);
            assert_eq!(state, before, "the public input is immutable");
        }
    }

    #[test]
    fn guiding_star_autoplay_keeps_resources_and_uses_the_same_body() {
        let (mut state, catalog, source) = guiding_star_parts(1, 100);
        state.energy = 0;
        state.stars = 0;
        let seeded = Xoshiro256StarStar::from_seed(17);
        state.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let mut events = Vec::new();

        crate::engine::play::autoplay_imitation_clone(&mut state, &catalog, source, &mut events)
            .unwrap();

        assert_eq!(state.energy, 0);
        assert_eq!(state.stars, 0);
        assert_eq!(state.monsters[0].hp, 87);
        assert_eq!(state.powers.value(PowerId::DrawNextTurn), 3);
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
    }

    #[test]
    fn guiding_star_terminal_attack_suppresses_delayed_draw() {
        let (mut state, catalog, source) = guiding_star_parts(0, 12);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);

        let next = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;

        assert!(next.history.over);
        assert_eq!(next.powers.value(PowerId::DrawNextTurn), 0);
    }

    #[test]
    fn guiding_star_malformed_rows_and_overflow_refuse_atomically() {
        for (upgrade, args, target, expected) in [
            (
                0,
                vec![CompiledArg::I(3)],
                Some(0),
                EngineRefusal::MalformedArgs("guiding_star_draw_next_turn_exact"),
            ),
            (
                0,
                vec![CompiledArg::I(2)],
                None,
                EngineRefusal::TargetMismatch { required: true },
            ),
        ] {
            let (mut state, catalog, source) = guiding_star_parts(upgrade, 100);
            let before = state.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let mut events = Vec::new();
            let result = guiding_star_draw_next_turn_exact(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            });
            assert_eq!(result, Err(expected));
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let (mut state, catalog, source) = guiding_star_parts(0, 100);
        let before = state.clone();
        let mut forged = *catalog.spec(source.atom).unwrap();
        forged.row = crate::content_tables::card_row(CardId::StrikeRegent, 0).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            guiding_star_draw_next_turn_exact(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &forged,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[CompiledArg::I(2)],
                events: &mut events,
            }),
            Err(EngineRefusal::MalformedArgs(
                "guiding_star_draw_next_turn_exact source row"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let (mut state, catalog, source) = guiding_star_parts(1, 100);
        state
            .powers
            .set(PowerId::DrawNextTurn, SlotWire::Int, i32::MAX - 2);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let before = state.clone();
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            ),
            Err(EngineRefusal::CounterOverflow("draw next turn"))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn bundle_of_joy_uses_one_shuffle_and_singular_hand_cap_routing() {
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.reward_card_pool = Some(RewardPool::Regent);
        state.entropy_card_pool = Some(RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.next_card_uid = 100;

        let mut preview = state.clone();
        let shuffled =
            shuffle_generation_pool(&mut preview, &REGENT_COLORLESS_GENERATION_POOL_V1101).unwrap();
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::BundleOfJoy,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        for id in shuffled.iter().copied().take(3) {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        for uid in 0..9 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: source_atom,
                flags: 0,
            });
        }
        let spec = *catalog.spec(source_atom).unwrap();
        let args = [CompiledArg::I(3)];
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

        bundle_of_joy_exact(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Generation).counter, 49);
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Discard).len(), 2);
        let actual = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .chain(state.piles.get(PileId::Discard).as_slice())
            .filter(|card| card.uid >= 100)
            .map(|card| {
                assert_eq!(card.flags, 0);
                catalog.spec(card.atom).unwrap().identity.id
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, shuffled[..3]);
        assert_eq!(state.history.owner_generated_cards_combat, 3);
        assert_eq!(state.next_generated_hook_uid, 3);
    }

    #[test]
    fn manifest_authority_blocks_first_then_generates_one_fresh_card() {
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.reward_card_pool = Some(RewardPool::Regent);
        state.entropy_card_pool = Some(RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.next_card_uid = 100;

        let mut preview = state.clone();
        let shuffled =
            shuffle_generation_pool(&mut preview, &REGENT_COLORLESS_GENERATION_POOL_V1101).unwrap();
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::ManifestAuthority,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: shuffled[0],
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        for uid in 0..4 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: source_atom,
                flags: 0,
            });
        }
        let spec = *catalog.spec(source_atom).unwrap();
        let args = [CompiledArg::I(7)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 2,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };

        manifest_authority_exact(&mut ctx).unwrap();

        // One full 50-card shuffle: the same 49-draw advance the Bundle of
        // Joy body pins from this seed, because both consume exactly one
        // shuffle regardless of how much of the prefix they keep.
        assert_eq!(state.rng.get(RngStream::Generation).counter, 49);
        assert_eq!(state.block, 7);
        assert_eq!(state.history.card_block_gains, 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 5);
        assert_eq!(state.piles.get(PileId::Discard).len(), 0);
        let inserted = *state.piles.get(PileId::Hand).as_slice().last().unwrap();
        assert_eq!(inserted.uid, 100);
        assert_eq!(inserted.flags, 0);
        let identity = catalog.spec(inserted.atom).unwrap().identity;
        assert_eq!(identity.id, shuffled[0]);
        assert_eq!(identity.upgrade, 0);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert_eq!(state.next_generated_hook_uid, 1);
        // The Block gain precedes the generated insert, in Python's order.
        assert!(matches!(
            events[0],
            Event::PlayerBlockGained {
                amount: 7,
                block: 7
            }
        ));
        assert!(matches!(
            events[1],
            Event::CardResolved {
                uid: 100,
                pile: PileId::Hand
            }
        ));

        // Without the Regent generation provenance the body refuses before
        // touching any state, exactly like Bundle of Joy.
        let mut bare = HotState::at_defaults();
        bare.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        let mut bare_events = Vec::new();
        let mut bare_ctx = StepCtx {
            state: &mut bare,
            catalog: &catalog,
            spec: &spec,
            source_uid: 2,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut bare_events,
        };
        assert_eq!(
            manifest_authority_exact(&mut bare_ctx),
            Err(EngineRefusal::MalformedArgs("manifest_authority_exact"))
        );
    }

    #[test]
    fn upgraded_manifest_authority_raises_its_card_and_overflows_a_full_hand() {
        let seeded = Xoshiro256StarStar::from_seed(0);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.reward_card_pool = Some(RewardPool::Regent);
        state.entropy_card_pool = Some(RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.next_card_uid = 200;

        let mut preview = state.clone();
        let shuffled =
            shuffle_generation_pool(&mut preview, &REGENT_COLORLESS_GENERATION_POOL_V1101).unwrap();
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::ManifestAuthority,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: shuffled[0],
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        for uid in 0..10 {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: source_atom,
                flags: 0,
            });
        }
        let spec = *catalog.spec(source_atom).unwrap();
        let args = [CompiledArg::I(8)];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: 3,
            target: None,
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };

        manifest_authority_exact(&mut ctx).unwrap();

        assert_eq!(state.rng.get(RngStream::Generation).counter, 49);
        assert_eq!(state.block, 8);
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Discard).len(), 1);
        let inserted = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!(inserted.uid, 200);
        let identity = catalog.spec(inserted.atom).unwrap().identity;
        assert_eq!(identity.id, shuffled[0]);
        assert_eq!(identity.upgrade, 1);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
        assert!(matches!(
            events[1],
            Event::CardResolved {
                uid: 200,
                pile: PileId::Discard
            }
        ));
    }

    fn generation_state(seed: u64) -> HotState {
        let seeded = Xoshiro256StarStar::from_seed(seed);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.reward_card_pool = Some(RewardPool::Regent);
        state.entropy_card_pool = Some(RewardPool::Regent);
        state.set_spectrum_shift_generation_pool(true);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state
    }

    fn catalog_for_pool(source: CardIdentity, pool: &[CardId], upgrade: u8) -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.intern(source).unwrap();
        for id in pool {
            builder
                .intern(CardIdentity {
                    id: *id,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.build()
    }

    #[test]
    fn quasar_spends_then_freezes_three_options_and_skip_mints_nothing() {
        for owner in [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Defect,
            RewardPool::Necrobinder,
            RewardPool::Regent,
        ] {
            check_quasar_spends_then_freezes_three_options_and_skip_mints_nothing(owner);
        }
    }

    fn check_quasar_spends_then_freezes_three_options_and_skip_mints_nothing(owner: RewardPool) {
        let source = CardIdentity {
            id: CardId::Quasar,
            upgrade: 0,
            enchantment: None,
        };
        let catalog = catalog_for_pool(source, &REGENT_COLORLESS_GENERATION_POOL_V1101, 0);
        let mut state = generation_state(17);
        state.reward_card_pool = Some(owner);
        state.entropy_card_pool = Some(owner);
        state.set_spectrum_shift_generation_pool(false);
        state.stars = 2;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        let before = state.clone();

        let suspended = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let pending = suspended.pending.as_deref().unwrap();
        let record = pending.record(&suspended.frames).unwrap();
        assert_eq!(
            record.route().unwrap().1,
            crate::hot::PendingSelectionKind::Quasar
        );
        assert_eq!(record.selection_cards().len(), 3);
        let expected_actions = crate::engine::legal_actions(&suspended, &catalog);
        let mut action_buffer = crate::engine::LegalActionBuffer::new();
        assert_eq!(
            crate::engine::legal_actions_into(&suspended, &catalog, &mut action_buffer),
            expected_actions
        );
        assert_eq!(suspended.stars, 0);
        assert_eq!(suspended.rng.get(RngStream::Generation).counter, 49);
        assert_eq!(crate::engine::legal_actions(&suspended, &catalog).len(), 4);
        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        let mut expected_rng = Xoshiro256StarStar::from_seed(17);
        let mut expected_pool = REGENT_COLORLESS_GENERATION_POOL_V1101;
        expected_rng.shuffle(&mut expected_pool).unwrap();
        assert_eq!(
            wire.player["pending"],
            serde_json::json!([
                "quasar_select",
                wire.player["pending"][1],
                0,
                expected_pool[..3]
                    .iter()
                    .map(|id| serde_json::json!([id.as_str(), 0]))
                    .collect::<Vec<_>>()
            ])
        );
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );
        let reloaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        for id in REGENT_COLORLESS_GENERATION_POOL_V1101 {
            assert!(
                reloaded_catalog
                    .atom(&CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .is_some(),
                "a pending-only reload must retain Quasar's future replay closure: {id:?}"
            );
        }
        let reloaded = HotBoundary::from_canonical(&wire, &reloaded_catalog).unwrap();
        let reloaded_refusal = crate::engine::admission::admit(&wire, &reloaded, &reloaded_catalog)
            .expect_err("the complete pool still contains independently refused leaves");
        assert!(!reloaded_refusal.contains(
            crate::engine::admission::MissingCapability::ArgumentShape("Quasar generation closure")
        ));
        for malformed in [
            {
                let mut malformed = wire.clone();
                let duplicate = malformed.player["pending"][3][0].clone();
                malformed.player.get_mut("pending").unwrap()[3][1] = duplicate;
                malformed
            },
            {
                let mut malformed = wire.clone();
                malformed.player.get_mut("pending").unwrap()[2] = serde_json::json!(1);
                malformed
            },
            {
                let mut malformed = wire.clone();
                malformed.player.get_mut("pending").unwrap()[3]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!([expected_pool[3].as_str(), 0]));
                malformed
            },
        ] {
            assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());
        }

        let skipped = apply_action(
            &suspended,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(3),
            },
        )
        .unwrap()
        .state;
        assert!(skipped.pending.is_none());
        assert_eq!(skipped.history.owner_generated_cards_combat, 0);
        assert_eq!(skipped.next_card_uid, 2);
        assert_eq!(skipped.piles.get(PileId::Discard).as_slice()[0].uid, 1);

        let mut replay = skipped.clone();
        replay.stars = 2;
        let source = replay.piles.get_mut(PileId::Discard).make_mut().remove(0);
        replay.piles.get_mut(PileId::Hand).make_mut().push(source);
        let suspended_twice = apply_action(
            &replay,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            suspended_twice.rng.get(RngStream::Generation).counter,
            98,
            "the non-Exhaust source spends a second complete shuffle on replay"
        );

        let suspended_again = apply_action(
            &before,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let chosen_identity = catalog
            .spec(
                suspended_again
                    .pending
                    .as_deref()
                    .unwrap()
                    .record(&suspended_again.frames)
                    .unwrap()
                    .selection_cards()[1]
                    .atom,
            )
            .unwrap()
            .identity;
        let chosen = apply_action(
            &suspended_again,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap()
        .state;
        assert!(chosen.pending.is_none());
        assert_eq!(chosen.history.owner_generated_cards_combat, 1);
        assert_eq!(chosen.next_card_uid, 3);
        assert_eq!(
            catalog
                .spec(chosen.piles.get(PileId::Hand).as_slice()[0].atom)
                .unwrap()
                .identity,
            chosen_identity,
        );
        assert_eq!(
            apply_action(
                &suspended_again,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(4),
                },
            )
            .unwrap_err(),
            EngineRefusal::MalformedArgs("selection option index")
        );
    }

    #[test]
    fn upgraded_quasar_preserves_option_level_and_full_hand_overflow() {
        for owner in [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Defect,
            RewardPool::Necrobinder,
            RewardPool::Regent,
        ] {
            check_upgraded_quasar_preserves_option_level_and_full_hand_overflow(owner);
        }
    }

    fn check_upgraded_quasar_preserves_option_level_and_full_hand_overflow(owner: RewardPool) {
        let source = CardIdentity {
            id: CardId::Quasar,
            upgrade: 1,
            enchantment: None,
        };
        let catalog = catalog_for_pool(source, &REGENT_COLORLESS_GENERATION_POOL_V1101, 1);
        let mut state = generation_state(29);
        state.reward_card_pool = Some(owner);
        state.entropy_card_pool = Some(owner);
        state.set_spectrum_shift_generation_pool(false);
        state.stars = 2;
        state.next_card_uid = 100;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        let mut suspended = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap_or_else(|e| panic!("{owner:?}: {e:?}"))
        .state;
        assert!(
            suspended
                .pending
                .as_deref()
                .unwrap()
                .record(&suspended.frames)
                .unwrap()
                .selection_cards()
                .iter()
                .all(|card| catalog.spec(card.atom).unwrap().identity.upgrade == 1)
        );
        for uid in 10..20 {
            suspended
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(HotCard {
                    uid,
                    atom: catalog.atom(&source).unwrap(),
                    flags: 0,
                });
        }
        let chosen = apply_action(
            &suspended,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap_or_else(|e| panic!("{owner:?}: {e:?}"))
        .state;
        let generated = chosen
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == 100)
            .expect("a full owner hand routes the chosen fresh card to Discard");
        assert_eq!(catalog.spec(generated.atom).unwrap().identity.upgrade, 1);

        let mut ended = suspended;
        ended.history.over = true;
        let pending = ended.pending.clone().unwrap();
        let record = pending.record(&ended.frames).unwrap().to_owned();
        let selection_cards = record.selection_cards.clone();
        let kind = record.selection_kind.unwrap();
        let active = *ended
            .piles
            .get(record.source_pile)
            .as_slice()
            .iter()
            .find(|card| card.uid == record.uid)
            .unwrap();
        let before = ended.clone();
        let mut events = Vec::new();
        crate::engine::selection::apply_special_option(
            &mut ended,
            &catalog,
            kind,
            active,
            &record,
            &selection_cards,
            0,
            &mut events,
        )
        .unwrap();
        assert_eq!(ended, before, "an ending fight cannot mint the choice");
        assert!(events.is_empty());
    }

    #[test]
    fn largesse_uses_selected_epochs_but_owner_rng_uid_history_and_pile() {
        let source = CardIdentity {
            id: CardId::Largesse,
            upgrade: 1,
            enchantment: None,
        };
        let catalog = catalog_for_pool(source, &LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101, 1);
        let mut state = generation_state(23);
        state.next_card_uid = 2;
        state.multiplayer_ally_key = 1;
        // The remote target's exact Colorless epochs are sufficient for this
        // chosen body even when the local player's broader splash epoch set is
        // absent. Admission separately refuses the whole entry because the
        // still-legal self target would be unrepresentable.
        state.fully_unlocked_card_pool_epochs = false;
        let ally = MultiplayerAllyState {
            key: 1,
            fully_unlocked_colorless_epochs: true,
            ..MultiplayerAllyState::default()
        };
        state.fanouts.set_multiplayer_ally(ally.clone());
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        let before = state.clone();
        let wire = HotBoundary::to_canonical(&state, &catalog);
        assert_eq!(
            wire.player["multiplayer_allies"],
            serde_json::json!([{
                "type": "MultiplayerAllyState",
                "fields": {
                    "key": 1,
                    "colorless_unlock_epochs": [
                        "COLORLESS1_EPOCH",
                        "COLORLESS2_EPOCH",
                        "COLORLESS3_EPOCH",
                        "COLORLESS4_EPOCH",
                        "COLORLESS5_EPOCH",
                    ],
                },
            }])
        );
        assert_eq!(HotBoundary::from_canonical(&wire, &catalog).unwrap(), state);
        let remote_only_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        for id in LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101 {
            assert!(
                remote_only_catalog
                    .atom(&CardIdentity {
                        id,
                        upgrade: 1,
                        enchantment: None,
                    })
                    .is_some(),
                "the remote selected-player pool must not depend on local splash epochs: {id:?}"
            );
        }
        for malformed in [
            {
                let mut malformed = wire.clone();
                malformed.player.get_mut("multiplayer_allies").unwrap()[0]["fields"]
                    ["colorless_unlock_epochs"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                malformed
            },
            {
                let mut malformed = wire.clone();
                malformed.player.get_mut("multiplayer_allies").unwrap()[0]["fields"]
                    ["colorless_unlock_epochs"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1);
                malformed
            },
        ] {
            assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());
        }

        let resolved = apply_action(
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
        assert_eq!(resolved.rng.get(RngStream::Generation).counter, 61);
        assert_eq!(resolved.history.owner_generated_cards_combat, 1);
        assert_eq!(resolved.next_card_uid, 3);
        assert_eq!(resolved.fanouts.multiplayer_ally(), &ally);
        let generated = resolved.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(generated.uid, 2);
        assert_eq!(catalog.spec(generated.atom).unwrap().identity.upgrade, 1);
        assert_eq!(resolved.piles.get(PileId::Discard).as_slice()[0].uid, 1);

        let mut unrelated_entropy = before.clone();
        unrelated_entropy.entropy_card_pool = Some(RewardPool::Ironclad);
        let resolved = apply_action(
            &unrelated_entropy,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .expect("Largesse does not read the owner-local entropy pool")
        .state;
        assert_eq!(resolved.rng.get(RngStream::Generation).counter, 61);

        let mut missing_epochs = before.clone();
        missing_epochs
            .fanouts
            .set_multiplayer_ally(MultiplayerAllyState {
                key: 1,
                fully_unlocked_colorless_epochs: false,
                ..ally
            });
        let snapshot = missing_epochs.clone();
        assert_eq!(
            apply_action(
                &missing_epochs,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: Some(1),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_err(),
            EngineRefusal::MalformedArgs("largesse_exact target pool")
        );
        assert_eq!(missing_epochs, snapshot);

        let missing_local_epochs = before;
        let snapshot = missing_local_epochs.clone();
        assert_eq!(
            apply_action(
                &missing_local_epochs,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_err(),
            EngineRefusal::MalformedArgs("largesse_exact target pool")
        );
        assert_eq!(missing_local_epochs, snapshot);

        let mut solo = generation_state(41);
        solo.next_card_uid = 2;
        solo.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        let snapshot = solo.clone();
        assert_eq!(
            apply_action(
                &solo,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_err(),
            EngineRefusal::CardNotPlayableSolo(CardId::Largesse)
        );
        assert_eq!(solo, snapshot);

        let mut ended = generation_state(43);
        ended.multiplayer_ally_key = 1;
        ended.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            fully_unlocked_colorless_epochs: true,
            ..MultiplayerAllyState::default()
        });
        ended.next_card_uid = 9;
        ended.history.over = true;
        ended.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 8,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        let state_snapshot = ended.clone();
        let mut events = vec![Event::CardResolved {
            uid: 77,
            pile: PileId::Discard,
        }];
        let events_snapshot = events.clone();
        {
            let spec = *catalog.spec(catalog.atom(&source).unwrap()).unwrap();
            let mut ctx = StepCtx {
                state: &mut ended,
                catalog: &catalog,
                spec: &spec,
                source_uid: 8,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut events,
            };
            largesse_exact(&mut ctx).unwrap();
        }
        assert_eq!(ended, state_snapshot);
        assert_eq!(events, events_snapshot);
    }

    /// #3285: the frozen multiplayer Colorless table is
    /// `largesse_colorless_pool` at the fully-unlocked profile, card for card
    /// in native (RNG-significant) order — the licence for drawing the derived
    /// pool under a recorded partial profile.
    #[test]
    fn largesse_colorless_pool_reproduces_the_frozen_constant() {
        assert_eq!(
            crate::steps::neutral::largesse_colorless_pool(None).as_slice(),
            LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101.as_slice()
        );
    }

    /// Largesse aimed at the LOCAL Player draws the local recorded profile's
    /// multiplayer Colorless pool (#3285), on both profiles; aimed at the
    /// remote Player it draws the remote's exact fully-unlocked pool whatever
    /// the local profile is.
    ///
    /// `Largesse/<OnPlay>d__3::MoveNext` `0x3a90e4`: the TARGET Player's
    /// `UnlockState` into `GetUnlockedCards` (IL_00b3-IL_00f7), one
    /// `GetDistinctForCombat` shuffle on the SOURCE owner's
    /// `CombatCardGeneration` (IL_00fc-IL_0112), `FirstOrDefault` (IL_0117).
    /// The partial profile hides `COLORLESS5_EPOCH`, so the local pool loses
    /// its rows and the shuffle consumes fewer draws than the 61 of the full
    /// 62-row pool. Until #3285 the local target refused on any partial
    /// profile (`largesse_exact target pool`).
    #[test]
    fn largesse_local_target_draws_the_recorded_profile_colorless_pool() {
        let source = CardIdentity {
            id: CardId::Largesse,
            upgrade: 0,
            enchantment: None,
        };
        let mut hidden_profile: Vec<&'static str> =
            crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
                .iter()
                .copied()
                .filter(|epoch| *epoch != "COLORLESS5_EPOCH")
                .collect();
        hidden_profile.sort_unstable();
        let full = LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101.to_vec();
        for profile in [Some(hidden_profile), None] {
            let partial = profile.is_some();
            let mut builder = CatalogBuilder::new();
            if let Some(profile) = profile {
                builder.set_splash_unlock_epochs(profile);
            }
            builder.intern(source).unwrap();
            for id in LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101 {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            let catalog = builder.build();
            let local = catalog.largesse_local_colorless_pool();
            assert_eq!(local == full, !partial);
            for gated in [CardId::Anointed, CardId::Calamity] {
                assert!(full.contains(&gated));
                assert_eq!(!local.contains(&gated), partial, "{gated:?}");
            }

            let mut state = generation_state(29);
            state.next_card_uid = 2;
            state.multiplayer_ally_key = 1;
            state.fully_unlocked_card_pool_epochs = !partial;
            state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
                key: 1,
                fully_unlocked_colorless_epochs: true,
                ..MultiplayerAllyState::default()
            });
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: catalog.atom(&source).unwrap(),
                flags: 0,
            });
            for (target, pool) in [(0u8, &local), (1u8, &full)] {
                let mut preview = {
                    let live = state.rng.get(RngStream::Generation);
                    Xoshiro256StarStar {
                        words: live.words,
                        counter: live.counter,
                    }
                };
                let mut shuffled = pool.clone();
                preview.shuffle(&mut shuffled).unwrap();
                let resolved = apply_action(
                    &state,
                    &catalog,
                    &Action::Play {
                        uid: 1,
                        target: Some(target),
                        selection: SelectionRef::NONE,
                    },
                )
                .unwrap_or_else(|refusal| panic!("partial={partial} target={target}: {refusal:?}"))
                .state;
                assert_eq!(
                    resolved.rng.get(RngStream::Generation).counter,
                    u64::try_from(pool.len() - 1).unwrap(),
                    "partial={partial} target={target}"
                );
                let generated = resolved.piles.get(PileId::Hand).as_slice()[0];
                assert_eq!(
                    catalog.spec(generated.atom).unwrap().identity,
                    CardIdentity {
                        id: shuffled[0],
                        upgrade: 0,
                        enchantment: None,
                    },
                    "partial={partial} target={target}"
                );
            }
        }
    }

    #[test]
    fn largesse_fresh_the_ball_enters_with_zero_growth_and_is_immediately_playable() {
        let source = CardIdentity {
            id: CardId::Largesse,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(source).unwrap();
        for id in LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101 {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder
            .intern(CardIdentity {
                id: CardId::TheBall,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let seed = (0..10_000)
            .find(|seed| {
                let mut probe = generation_state(*seed);
                shuffle_generation_pool(&mut probe, &LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101)
                    .is_ok_and(|pool| pool[0] == CardId::TheBall)
            })
            .expect("the fixed Largesse pool has a deterministic The Ball prefix");
        let mut state = generation_state(seed);
        state.next_card_uid = 2;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            shuffle_rng: Some(RngStreamState {
                words: [31, 32, 33, 34],
                counter: 0,
            }),
            ..MultiplayerAllyState::default()
        });
        state.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: [41, 42, 43, 44],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&source).unwrap(),
            flags: 0,
        });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));

        let generated = apply_action(
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
        let ball = *generated
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::TheBall))
            })
            .unwrap();
        assert_ne!(ball.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(
            generated.card_states.get(ball.uid),
            CardInstanceState::default()
        );
        assert!(
            crate::engine::legal_actions(&generated, &catalog)
                .iter()
                .any(|action| matches!(action, Action::Play { uid, .. } if *uid == ball.uid))
        );

        let routed = apply_action(
            &generated,
            &catalog,
            &Action::Play {
                uid: ball.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            routed.fanouts.multiplayer_ally().draw[0].the_ball_growth(),
            Some(10)
        );
    }

    #[test]
    fn largesse_replays_full_shuffles_and_autoplay_rolls_the_remote_singleton() {
        let source = CardIdentity {
            id: CardId::Largesse,
            upgrade: 0,
            enchantment: None,
        };
        let catalog = catalog_for_pool(source, &LARGESSE_MULTIPLAYER_COLORLESS_POOL_V1101, 0);
        let atom = catalog.atom(&source).unwrap();

        let mut serial = generation_state(31);
        serial.multiplayer_ally_key = 1;
        serial.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            fully_unlocked_colorless_epochs: true,
            ..MultiplayerAllyState::default()
        });
        serial.next_card_uid = 100;
        for uid in 1..=2 {
            serial.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        for uid in 1..=2 {
            serial = apply_action(
                &serial,
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
        assert_eq!(serial.rng.get(RngStream::Generation).counter, 122);
        assert_eq!(serial.history.owner_generated_cards_combat, 2);
        assert_eq!(serial.next_card_uid, 102);

        let mut autoplay = generation_state(37);
        autoplay.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        autoplay.fully_unlocked_card_pool_epochs = false;
        autoplay.multiplayer_ally_key = 1;
        autoplay.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            fully_unlocked_colorless_epochs: true,
            ..MultiplayerAllyState::default()
        });
        let target_rng = Xoshiro256StarStar::from_seed(41);
        autoplay.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: target_rng.words,
                counter: target_rng.counter,
            },
        );
        autoplay.next_card_uid = 100;
        for uid in 10..20 {
            autoplay
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(HotCard {
                    uid,
                    atom,
                    flags: 0,
                });
        }
        let active = HotCard {
            uid: 1,
            atom,
            flags: 0,
        };
        autoplay.piles.get_mut(PileId::Play).make_mut().push(active);
        crate::engine::play::autoplay_imitation_clone(
            &mut autoplay,
            &catalog,
            active,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(autoplay.rng.get(RngStream::Targets).counter, 1);
        assert_eq!(autoplay.rng.get(RngStream::Generation).counter, 61);
        assert_eq!(autoplay.piles.get(PileId::Hand).len(), 10);
        assert!(
            autoplay
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 100),
            "AutoPlay's generated card overflows the full owner Hand"
        );
    }

    /// Every kind this family owns and does not claim must refuse **by its own
    /// kind**, through the real dispatch.
    ///
    /// This is the "refusal stubs not weakened" property a wave review checks,
    /// made mechanical: a body silently returning `Ok(())` — the one way an
    /// unported kind could corrupt a fight without the differential noticing,
    /// because the card would then play as if the step were absent — fails
    /// here. It is written against `IMPLEMENTED` rather than against a
    /// hardcoded list, so the kind that lands a real body next simply drops
    /// out of the checked set instead of turning this test red.
    #[test]
    fn every_unclaimed_kind_refuses_by_its_own_kind() {
        let (mut state, catalog) = fixture();
        let atom = state.piles.get(PileId::Hand).as_slice()[0].atom;
        let spec = *catalog.spec(atom).unwrap();
        let mut events = Vec::new();
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
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
                "{:?} neither implements its Python branch nor refuses by name",
                kind.as_str()
            );
        }
    }

    /// This family's claim agrees with the generated dispatch tree's view of
    /// it.
    ///
    /// The first half checks that every claimed kind is one of this file's
    /// stubs (Bundle of Joy since #1374, Manifest Authority since #1336's
    /// batch-3 wave); the nine unclaimed kinds each still reach a primitive
    /// the engine lacks (see [`IMPLEMENTED`]'s note).
    ///
    /// The second half is the cross-check [`OWNED`] owes: `is_implemented`
    /// scans every family's registry, so a kind listed here and *also* claimed
    /// by another family — the one way two wave PRs can collide despite
    /// disjoint files — shows up as a claim this family cannot account for.
    #[test]
    fn the_family_claims_only_its_ported_kinds_and_never_a_foreign_kind() {
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
    fn tutor_rows_are_exactly_two_implemented_multiplayer_selectors() {
        let rows = crate::content_tables::CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::TutorExact)
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| {
            matches!(row.id, CardId::Tutor)
                && row.upgrade <= 1
                && row.targeted
                && matches!(row.target_type, "AnyAlly")
                && row.selects
        }));
        assert!(IMPLEMENTED.contains(&StepKind::TutorExact));
        assert_eq!((rows[0].cost, rows[1].cost), (1, 0));
        assert!(rows.iter().all(|row| matches!(row.steps, [step] if
            step.kind == StepKind::TutorExact && step.args.is_empty())));

        let mut builder = CatalogBuilder::new();
        let atoms = rows
            .iter()
            .map(|row| {
                builder
                    .intern_reachable(CardIdentity {
                        id: row.id,
                        upgrade: row.upgrade,
                        enchantment: None,
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let catalog = builder.build();
        assert!(catalog.requires_action_replay());
        assert!(
            atoms
                .iter()
                .all(|atom| catalog.spec(*atom).unwrap().solo_unplayable)
        );
        assert!(
            crate::content_tables::GENERATION_POOLS
                .iter()
                .all(|(_, pool)| !pool.contains(&CardId::Tutor))
        );
    }

    fn tutor_parts(draw_count: u32) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let tutor_atom = builder
            .intern_reachable(CardIdentity {
                id: CardId::Tutor,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend_atom = builder
            .intern_reachable(CardIdentity {
                id: CardId::DefendRegent,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: tutor_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.multiplayer_ally_key = 1;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        for offset in 0..draw_count {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2 + offset,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        state.next_card_uid = 2 + draw_count;
        let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        let source = rebuilt.piles.get(PileId::Hand).as_slice()[0];
        (rebuilt, rebuilt_catalog, source)
    }

    #[test]
    fn tutor_local_multi_draw_parks_with_uid_actions_roundtrips_and_resumes() {
        let (state, catalog, source) = tutor_parts(2);
        let initial_wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(
            crate::engine::admit(&initial_wire, &state, &catalog),
            Ok(())
        );
        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(parked.energy, 2);
        assert_eq!(parked.piles.get(PileId::Draw).len(), 2);
        let actions = crate::engine::legal_actions(&parked, &catalog);
        assert_eq!(
            actions,
            vec![
                Action::Select {
                    answer: SelectionAnswer::CardUid(3)
                },
                Action::Select {
                    answer: SelectionAnswer::CardUid(2)
                },
            ]
        );
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(
            wire.player["pending"][2],
            serde_json::json!(["select", "draw", 1, 1, null, ["move", "hand", "bottom"]])
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        assert!(rebuilt_catalog.requires_action_replay());
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        assert_eq!(
            crate::engine::play::persisted_card_play_stack_is_exact(&rebuilt, &rebuilt_catalog),
            Ok(())
        );
        assert!(crate::boundary::action_replay_is_authenticated(
            &wire,
            &rebuilt,
            &rebuilt_catalog
        ));
        assert_eq!(
            crate::engine::admit(&wire, &rebuilt, &rebuilt_catalog),
            Ok(())
        );
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(3),
            },
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none());
        assert!(
            resumed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| card.uid == 3)
        );
        assert!(
            resumed
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );
        assert_eq!(resumed.piles.get(PileId::Draw).len(), 1);

        let mut shrunk = rebuilt.clone();
        shrunk
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .retain(|card| card.uid == 3);
        let before = shrunk.clone();
        assert!(HotBoundary::try_to_canonical(&shrunk, &rebuilt_catalog).is_err());
        assert_eq!(
            apply_action(
                &shrunk,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(3)
                },
            ),
            // A state the boundary cannot canonicalise publishes no legal
            // action, so this is a legality refusal and says so since #2473.
            Err(EngineRefusal::ActionNotLegal("public action legality"))
        );
        assert_eq!(shrunk, before);

        let mut duplicate_candidate = rebuilt.clone();
        duplicate_candidate.piles.get_mut(PileId::Draw).make_mut()[0].uid = 1;
        let before = duplicate_candidate.clone();
        assert!(HotBoundary::try_to_canonical(&duplicate_candidate, &rebuilt_catalog).is_err());
        assert_eq!(
            apply_action(
                &duplicate_candidate,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(3)
                },
            ),
            // As above: an uncanonicalisable state publishes no legal action.
            Err(EngineRefusal::ActionNotLegal("public action legality"))
        );
        assert_eq!(duplicate_candidate, before);

        let mut stale = rebuilt.clone();
        stale
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .retain(|card| card.uid != 3);
        let before = stale.clone();
        assert_eq!(
            apply_action(
                &stale,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(3)
                },
            ),
            // The selected card is gone from Draw, so the answer names nothing
            // the pending publishes — a legality refusal, since #2473.
            Err(EngineRefusal::ActionNotLegal("public action legality"))
        );
        assert_eq!(stale, before);

        let mut wrong_key = wire.clone();
        let frame = wrong_key
            .continuations
            .iter_mut()
            .find(|frame| frame.fields.contains_key("choice"))
            .unwrap();
        frame
            .fields
            .insert("choice".to_owned(), serde_json::json!(1));
        let wrong_catalog = HotBoundary::catalog_from_canonical(&wrong_key).unwrap();
        assert!(HotBoundary::from_canonical(&wrong_key, &wrong_catalog).is_err());

        let mut wrong_operation = wire.clone();
        wrong_operation.player.get_mut("pending").unwrap()[2] =
            serde_json::json!(["select", "discard", 1, 1, null, ["move", "hand", "bottom"]]);
        let wrong_catalog = HotBoundary::catalog_from_canonical(&wrong_operation).unwrap();
        assert!(HotBoundary::from_canonical(&wrong_operation, &wrong_catalog).is_err());

        let mut wrong_identity = wire.clone();
        wrong_identity.piles.get_mut("play").unwrap()[0].id =
            CardId::DefendRegent.as_str().to_owned();
        let wrong_catalog = HotBoundary::catalog_from_canonical(&wrong_identity).unwrap();
        assert!(HotBoundary::from_canonical(&wrong_identity, &wrong_catalog).is_err());
    }

    #[test]
    fn tutor_refuses_duplicate_uids_dead_remote_solo_and_nonplayer_targets_atomically() {
        let (mut duplicate, catalog, source) = tutor_parts(2);
        duplicate.piles.get_mut(PileId::Draw).make_mut()[1].uid = 2;
        let before = duplicate.clone();
        assert!(
            apply_action(
                &duplicate,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .is_err()
        );
        assert_eq!(duplicate, before);

        let (mut dead, catalog, source) = tutor_parts(0);
        dead.fanouts.multiplayer_ally_mut().alive = false;
        let before = dead.clone();
        for target in [Some(0), Some(1), Some(2)] {
            assert!(
                apply_action(
                    &dead,
                    &catalog,
                    &Action::Play {
                        uid: source.uid,
                        target,
                        selection: SelectionRef::NONE,
                    },
                )
                .is_err()
            );
            assert_eq!(dead, before);
        }

        let (mut pet, catalog, _) = tutor_parts(0);
        pet.fanouts.set_osty(Some((3, 5))).unwrap();
        let wire = HotBoundary::try_to_canonical(&pet, &catalog).unwrap();
        assert!(
            crate::engine::admit(&wire, &pet, &catalog)
                .unwrap_err()
                .contains(crate::engine::admission::MissingCapability::ArgumentShape(
                    "manual AnyAlly target roster"
                ))
        );
    }

    #[test]
    fn tutor_local_uid_zero_is_a_valid_selection_identity() {
        let (mut state, catalog, source) = tutor_parts(2);
        state.piles.get_mut(PileId::Draw).make_mut()[0].uid = 0;
        let initial = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&initial).unwrap();
        let rebuilt = HotBoundary::from_canonical(&initial, &rebuilt_catalog).unwrap();
        let parked = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let parked_wire = HotBoundary::try_to_canonical(&parked, &rebuilt_catalog).unwrap();
        let parked_catalog = HotBoundary::catalog_from_canonical(&parked_wire).unwrap();
        let parked = HotBoundary::from_canonical(&parked_wire, &parked_catalog).unwrap();
        assert!(crate::boundary::action_replay_is_authenticated(
            &parked_wire,
            &parked,
            &parked_catalog
        ));
        assert!(
            crate::engine::legal_actions(&parked, &parked_catalog).contains(&Action::Select {
                answer: SelectionAnswer::CardUid(0),
            })
        );
        let resumed = apply_action(
            &parked,
            &parked_catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(0),
            },
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none());
        assert!(
            resumed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| card.uid == 0)
        );
    }

    #[test]
    fn tutor_local_empty_singleton_and_full_hand_routes_are_exact() {
        let (state, catalog, source) = tutor_parts(0);
        let empty = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(empty.pending.is_none());
        assert!(empty.piles.get(PileId::Hand).is_empty());

        let (state, catalog, source) = tutor_parts(1);
        let singleton = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(singleton.pending.is_none());
        assert_eq!(singleton.piles.get(PileId::Hand).as_slice()[0].uid, 2);

        let (mut full, catalog, source) = tutor_parts(1);
        full.piles.get_mut(PileId::Hand).make_mut().clear();
        full.piles.get_mut(PileId::Play).make_mut().push(source);
        let filler = full.piles.get(PileId::Draw).as_slice()[0];
        full.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..10).map(|offset| HotCard {
                uid: 10 + offset,
                ..filler
            }));
        full.next_card_uid = 20;
        let spec = *catalog.spec(source.atom).unwrap();
        assert!(
            !begin_tutor_exact(&mut StepCtx {
                state: &mut full,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut Vec::new(),
            })
            .unwrap()
        );
        assert_eq!(full.piles.get(PileId::Hand).len(), 10);
        assert!(
            full.piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 2)
        );
    }

    #[test]
    fn tutor_remote_empty_and_singleton_are_choice_free_and_cow_exact() {
        let (state, catalog, source) = tutor_parts(0);
        let empty = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(empty.pending.is_none());

        let (mut state, catalog, source) = tutor_parts(0);
        let identity = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        state.fanouts.multiplayer_ally_mut().draw =
            std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(identity).unwrap()]);
        let sibling = state.clone();
        let mut remote_location_only = state.clone();
        remote_location_only.fanouts.multiplayer_ally_mut().draw = std::sync::Arc::new(Vec::new());
        remote_location_only.fanouts.multiplayer_ally_mut().hand =
            std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(identity).unwrap()]);
        let before_key = HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .differential_digest();
        let remote_location_key = HotBoundary::try_to_canonical(&remote_location_only, &catalog)
            .unwrap()
            .differential_digest();
        assert_ne!(before_key, remote_location_key);
        let moved = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(moved.fanouts.multiplayer_ally().draw.is_empty());
        assert_eq!(moved.fanouts.multiplayer_ally().hand[0].identity, identity);
        assert_eq!(sibling.fanouts.multiplayer_ally().draw.len(), 1);
        let after_key = HotBoundary::try_to_canonical(&moved, &catalog)
            .unwrap()
            .differential_digest();
        assert_ne!(before_key, after_key);

        let (mut full, catalog, source) = tutor_parts(0);
        let dynamic_identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 1,
            enchantment: None,
        };
        let dynamic = MultiplayerAllyCard::sovereign_blade(dynamic_identity, 37).unwrap();
        {
            let ally = full.fanouts.multiplayer_ally_mut();
            ally.draw = std::sync::Arc::new(vec![dynamic]);
            ally.hand = std::sync::Arc::new(vec![
                MultiplayerAllyCard::immutable(identity).unwrap();
                crate::engine::draw::MAX_CARDS_IN_HAND
            ]);
        }
        let moved = apply_action(
            &full,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(moved.fanouts.multiplayer_ally().draw.is_empty());
        assert_eq!(moved.fanouts.multiplayer_ally().hand.len(), 10);
        assert_eq!(
            moved.fanouts.multiplayer_ally().discard.as_slice(),
            &[dynamic]
        );
        assert_eq!(
            moved.fanouts.multiplayer_ally().discard[0].sovereign_blade_damage(),
            Some(37)
        );

        let valid_wire = HotBoundary::try_to_canonical(&full, &catalog).unwrap();
        let mut malformed = valid_wire.clone();
        malformed.player.get_mut("multiplayer_allies").unwrap()[0]["fields"]["draw"][0][6][1] =
            serde_json::json!(-1);
        let before = malformed.clone();
        assert!(HotBoundary::catalog_from_canonical(&malformed).is_err());
        assert_eq!(malformed, before);
    }

    #[test]
    fn tutor_remote_many_refuses_before_spend_rng_or_cow_mutation() {
        let (mut state, catalog, source) = tutor_parts(0);
        let identity = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        state.fanouts.multiplayer_ally_mut().draw = std::sync::Arc::new(vec![
            MultiplayerAllyCard::immutable(identity).unwrap(),
            MultiplayerAllyCard::immutable(identity).unwrap(),
        ]);
        for target in [Some(0), Some(1)] {
            let before = state.clone();
            assert_eq!(
                apply_action(
                    &state,
                    &catalog,
                    &Action::Play {
                        uid: source.uid,
                        target,
                        selection: SelectionRef::NONE
                    },
                ),
                Err(EngineRefusal::ContinuationNotModeled)
            );
            assert_eq!(state, before);
        }
    }

    #[test]
    fn tutor_burst_rereads_live_draw_and_reparks_for_the_second_body() {
        let (mut state, _, _) = tutor_parts(3);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        let wire = HotBoundary::try_to_canonical(&state, &{
            let mut builder = CatalogBuilder::new();
            for card in PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
            {
                let id = if card.uid == 1 {
                    CardId::Tutor
                } else {
                    CardId::DefendRegent
                };
                builder
                    .intern_reachable(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.build()
        })
        .unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        let first = apply_action(
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
        let second = apply_action(
            &first,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(2),
            },
        )
        .unwrap()
        .state;
        assert!(second.pending.is_some());
        assert_eq!(second.piles.get(PileId::Hand).len(), 1);
        assert_eq!(second.piles.get(PileId::Draw).len(), 2);
        assert!(
            crate::engine::legal_actions(&second, &catalog)
                .iter()
                .all(|action| matches!(
                    action,
                    Action::Select {
                        answer: SelectionAnswer::CardUid(3 | 4)
                    }
                ))
        );
        let wire = HotBoundary::try_to_canonical(&second, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        crate::engine::admit(&wire, &rebuilt, &rebuilt_catalog).unwrap();
        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(4),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.piles.get(PileId::Hand).len(), 2);
        assert_eq!(completed.piles.get(PileId::Draw).len(), 1);
        assert_eq!(completed.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn tutor_autoplay_rolls_the_sole_remote_target_once_and_never_selects() {
        let (mut state, catalog, source) = tutor_parts(0);
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Draw).make_mut().push(source);
        let identity = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        state.fanouts.multiplayer_ally_mut().draw =
            std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(identity).unwrap()]);
        let seeded = Xoshiro256StarStar::from_seed(73);
        state.rng.set(
            crate::hot::RngStream::Targets,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        let before = state.rng.get(crate::hot::RngStream::Targets).counter;
        crate::engine::play::autoplay_draw_top(&mut state, &catalog, 1, &mut Vec::new()).unwrap();
        assert!(state.pending.is_none() && state.frames.is_empty());
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Targets).counter,
            before + 1
        );
        assert!(state.fanouts.multiplayer_ally().draw.is_empty());
        assert_eq!(state.fanouts.multiplayer_ally().hand[0].identity, identity);
        assert!(
            state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );
    }

    /// #3515: Huddle Up draws for each living teammate through
    /// `CardPileCmd.Draw` (`0x3a60bc` IL_0097), whose `<DrawInternal>d__21`
    /// (0x3e3a70 IL_002e) returns at `IsOverOrEnding`. While the combat is
    /// ending before the over latch neither the recipient walk nor the remote
    /// suffix draws; the Adaptable-vetoed control draws for both Players.
    #[test]
    fn huddle_up_draws_nothing_while_combat_is_ending_before_the_over_latch() {
        let defend = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let remote_card = MultiplayerAllyCard::immutable(defend).unwrap();
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.multiplayer_ally_key = 1;
        template.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            alive: true,
            draw: std::sync::Arc::new(vec![remote_card, remote_card]),
            ..MultiplayerAllyState::default()
        });
        template.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 3,
                atom,
                flags: 0,
            },
        ]);
        crate::engine::damage::assert_ending_window_gate(&template, "Huddle Up", |s, _| {
            huddle_up_sequence(s, &catalog, 1, &mut Vec::new())
        });
        crate::engine::damage::assert_ending_window_gate(&template, "Huddle Up remote", |s, _| {
            huddle_up_remote_suffix(s, &catalog, 1, &mut Vec::new())
        });
    }
}
