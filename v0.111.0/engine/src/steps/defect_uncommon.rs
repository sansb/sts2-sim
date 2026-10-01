//! Card-step bodies for the `content/cards/defect_uncommon.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
use crate::catalog::{CardIdentity, CardSpec, CardTargetType, Catalog, CompiledArg};
use crate::content_tables::{CardRarity, CardRow};
use crate::engine::cards::{
    bulk_transform_fixed_same_pile, inject_generated_free_this_turn_bottom,
    normalize_card_identities, shuffle_generation_slice,
};
use crate::engine::damage::{gain_powered_card_block, player_attack_from_card};
use crate::engine::draw::{CardDrawResult, DrawSource, draw_cards_into};
use crate::engine::play::resolved_energy_cost;
use crate::engine::{EngineRefusal, Event, Subject};
use crate::hot::{CARD_FLAG_LEGACY, HotCard, MAX_ORB_SLOTS, MultiplayerAllyState, OrbKind, PileId};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::CompactExact,
    StepKind::EnergySurgeExact,
    StepKind::GainCurrentEnergyExact,
    StepKind::HibernatePowerExact,
    StepKind::RandomOrbLoopExact,
    StepKind::ScrapeExact,
    StepKind::WhiteNoiseExact,
];

fn compact_program(catalog: &Catalog, spec: &CardSpec) -> Result<(i64, u8), EngineRefusal> {
    let (block, level) = match (spec.identity.id, spec.identity.upgrade, catalog.steps(spec)) {
        (
            CardId::Compact,
            level @ 0..=1,
            [
                crate::catalog::CompiledStep {
                    kind: StepKind::CompactExact,
                    args,
                },
            ],
        ) if matches!(
            catalog.args(*args),
            [CompiledArg::I(block), CompiledArg::I(arg_level)]
                if *block == 6 + i64::from(level) && *arg_level == i64::from(level)
        ) =>
        {
            (6 + i64::from(level), level)
        }
        _ => return Err(EngineRefusal::MalformedArgs("compact_exact program")),
    };
    if !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("compact_exact program"));
    }
    Ok((block, level))
}

fn exact_active_compact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<HotCard, EngineRefusal> {
    let mut matches = PileId::ALL.into_iter().flat_map(|pile| {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(move |card| card.uid == source_uid)
    });
    let Some(source) = matches.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let count = 1 + matches.count();
    if count != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: count,
        });
    }
    if source.flags & CARD_FLAG_LEGACY != 0
        || catalog.spec(source.atom) != Some(spec)
        || compact_program(catalog, spec).is_err()
    {
        return Err(EngineRefusal::MalformedArgs("compact_exact source"));
    }
    Ok(source)
}

fn fuel_identity(catalog: &Catalog, level: u8) -> Result<CardIdentity, EngineRefusal> {
    let identity = CardIdentity {
        id: CardId::Fuel,
        upgrade: level,
        enchantment: None,
    };
    let spec = catalog
        .atom(&identity)
        .and_then(|atom| catalog.spec(atom))
        .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
    let expected_energy = 1 + i64::from(level);
    if spec.identity != identity
        || crate::content_tables::card_row(CardId::Fuel, level) != Some(spec.row)
        || spec.cost != 0
        || !spec.is_skill
        || !spec.exhausts
        || spec.targeted
        || !matches!(
            catalog.steps(spec),
            [crate::catalog::CompiledStep { kind: StepKind::Energy, args }]
                if catalog.args(*args) == [CompiledArg::I(expected_energy)]
        )
    {
        return Err(EngineRefusal::MalformedArgs("compact_exact Fuel"));
    }
    Ok(identity)
}

/// `Compact/<OnPlay>d__7` RVA `0x393740` awaits `CreatureCmd.GainBlock`
/// (IL_00c6) and then one plural `CardCmd.Transform` (IL_01ca), whose
/// `<Transform>d__13` (`0x3e0ae0`) returns at `IsEnding` (IL_0032-0037). So the
/// transform tests the shared IsEnding projection, not `history.over` (#3515).
fn execute_compact(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    block: i64,
    level: u8,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    exact_active_compact(state, catalog, source_uid, spec)?;
    gain_powered_card_block(state, catalog, spec, block, events)?;
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    normalize_card_identities(state)?;
    exact_active_compact(state, catalog, source_uid, spec)?;
    let frozen = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| {
            let candidate = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            Ok(candidate
                .is_status
                .then(|| (*card, state.card_states.get(card.uid))))
        })
        .collect::<Result<Vec<_>, EngineRefusal>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let replacement = fuel_identity(catalog, level)?;
    bulk_transform_fixed_same_pile(state, catalog, PileId::Hand, &frozen, replacement, events)?;
    Ok(())
}

/// Validate Compact's complete Block-plus-bulk-transform body before Energy,
/// source movement, CardPlayed listeners, or event publication.
pub(crate) fn preflight_compact_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    let (block, level) = compact_program(catalog, spec)?;
    exact_active_compact(state, catalog, source_uid, spec)?;
    let mut probe = state.clone();
    execute_compact(
        &mut probe,
        catalog,
        source_uid,
        spec,
        block,
        level,
        &mut Vec::new(),
    )
}

/// `compact_exact` — Compact's powered Block and exact plural Status
/// Transform.
///
/// Python `_b232_compact_exact` (frozen, deleted #2827) read Discard; that was
/// a copy error the `.mcr` census exposed (#3052), and the IL below governs.
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Compact/<OnPlay>d__7::MoveNext` RVA `0x393740` awaits powered Block 6/7,
/// then snapshots the owner's HAND in pile order: `IL_0124 ldc.i4.2` feeds
/// `IL_012b call GetPile`, and `PileType.Hand` is the constant 2 (`Draw` 1,
/// `Discard` 3). Filter `<>c::<OnPlay>b__7_0` RVA `0x393726` keeps a card
/// iff it is non-null, `IsTransformable`, and `Type == 4` (`CardType.Status`).
/// It creates fresh Fuel at the Compact level per candidate
/// (`IL_0178-01a5`) and invokes one plural `CardCmd.Transform` (`IL_01ca`).
/// In a combat pile `IsTransformable` is true even for Eternal cards
/// (`CardModel::get_IsTransformable` RVA `0x7ce20`). The active Compact sits
/// in Play, so it is never its own candidate, and Discard is untouched.
///
/// The shared bulk command removes the complete physical batch, then inserts
/// each Fuel at the index its Status held once the EARLIER candidates were
/// removed (`IndexOf` precedes each `RemoveFromCurrentPile` in one loop,
/// `CardCmd/<Transform>d__13` `IL_0121-018c`; #3199), so leading Fuels
/// gather ahead of the retained cards. It records generation for every result before the first
/// generated hook, and dispatches the synchronous admitted hook closure
/// serially without RNG or source aliasing. Hand capacity is never consulted:
/// `CardCmd/<Transform>d__13::MoveNext` RVA `0x3e0ae0` places every
/// non-Deck replacement with a raw `CardPile::AddInternal(card, index)`
/// (`IL_03e2-03f1`), not through `CardPileCmd.Add`'s ten-card overflow, and
/// every original has already left Hand (`RemoveFromCurrentPile`, `IL_018c`),
/// so Hand ends exactly as large as it began. The later Hand-only branch
/// (`IL_05ea-06a9`) is presentation only: it is skipped under
/// `NonInteractiveMode.IsOn`, and otherwise re-targets the card node and
/// withdraws the original from the UI play queue; the engine resolves each
/// play as one atomic action and holds no such queue.
/// The native pile-change callback is inert only because all three
/// current-build listeners are relics and R2 admits none; the focused
/// listener-census test makes that a derived seam.
pub(crate) fn compact_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (block, level) = compact_program(ctx.catalog, ctx.spec)?;
    if ctx.args != [CompiledArg::I(block), CompiledArg::I(i64::from(level))]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("compact_exact"));
    }
    exact_active_compact(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    let mut probe = ctx.state.clone();
    let mut produced = Vec::new();
    execute_compact(
        &mut probe,
        ctx.catalog,
        ctx.source_uid,
        ctx.spec,
        block,
        level,
        &mut produced,
    )?;
    *ctx.state = probe;
    ctx.events.extend(produced);
    Ok(())
}

/// `("energy_surge_exact", amount)` — serial Energy for every living Player
/// on the owner's side.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the exact carrier/operand
/// and calls `_gain_energy_to_all_allies` for the serial recipient
/// loop. The current-build IL below independently binds that oracle to native.
///
/// Current v0.111.0 IL (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// pins the exact body at `EnergySurge/<OnPlay>d__9::MoveNext` RVA
/// `0x39b948`. `GetTeammatesOf` (`0x137397`) returns the complete same-side
/// creature list; predicate `0x39b932` retains only non-null, alive Players,
/// and the body awaits `PlayerCmd.GainEnergy` (`0x3ee8a0`) for each in order.
/// The ctor/constraint/vars/upgrade RVAs
/// `0xde86f`/`0xde87c`/`0xde894`/`0xde8e7` pin the cost-one,
/// MultiplayerOnly, AllAllies, Exhausting Skill and amounts 2/3.
///
/// Unit C's exact quotient contains the owner and one stable remote Player.
/// Unknown teammate energy observers are rejected by the boundary, leaving
/// only recipient-local NoEnergyGain and the native cap, so canonical owner
/// then remote order commutes with the unrepresented absolute native order.
/// Rehearse the complete loop before applying it so no recipient prefix can
/// escape a later representability refusal.
pub(crate) fn energy_surge_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::EnergySurge, 0) => 2,
        (CardId::EnergySurge, 1) => 3,
        _ => return Err(EngineRefusal::MalformedArgs("energy_surge_exact")),
    };
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs("energy_surge_exact program"));
    };
    if ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::EnergySurgeExact
        || ctx.catalog.args(program.args) != [CompiledArg::I(i64::from(expected))]
        || ctx.args != [CompiledArg::I(i64::from(expected))]
    {
        return Err(EngineRefusal::MalformedArgs("energy_surge_exact"));
    }
    let matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| ctx.state.piles.get(pile).as_slice())
        .filter(|card| card.uid == ctx.source_uid)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: matches.len(),
        });
    }
    let source = matches[0];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "energy_surge_exact physical source",
        ));
    }

    fn apply(state: &mut crate::hot::HotState, amount: i32) -> Result<(), EngineRefusal> {
        let recipients = crate::engine::allies::living_keys(state).collect::<Vec<_>>();
        for key in recipients {
            crate::engine::allies::gain_energy(state, key, amount)?;
        }
        Ok(())
    }

    let mut probe = ctx.state.clone();
    apply(&mut probe, expected)?;
    apply(ctx.state, expected)
}

/// Double Energy's live-energy snapshot and gain.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827). The body snapshots the energy remaining
/// after this card paid its cost and adds that amount when positive. The
/// NoEnergyGain is boundary-representable and ledger-authenticated, but its
/// independent admission wall makes the modifier branch unreachable until
/// every local gain site is modeled (the same reduction used by
/// [`super::shared::energy`]). Combat-over suppresses this exact command.
///
/// `DoubleEnergy/<OnPlay>d__5` RVA `0x399ba0` awaits `PlayerCmd.GainEnergy`
/// (IL_00b9), and `<GainEnergy>d__3` (`0x3ee8a0`) returns at `IsEnding`
/// (IL_0035-003c): the shared IsEnding projection, not `history.over` (#3515).
pub(crate) fn gain_current_energy_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.spec.identity.id != CardId::DoubleEnergy
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
        || !ctx.args.is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("gain_current_energy_exact"));
    }
    if crate::engine::damage::damage_combat_is_ending(ctx.state) || ctx.state.energy <= 0 {
        return Ok(());
    }
    let amount = ctx.state.energy;
    ctx.state.energy = ctx
        .state
        .energy
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("gain_current_energy_exact"))?;
    Ok(())
}

/// Python authority: `_run_steps_inner` (frozen Python, deleted #2827) authenticates and installs the
/// stack through `_modify_hibernate_amount`; the captured turn-start
/// reader begins at `_after_turn_start_hand_draw`.
///
/// Unkeyed current-build `AfterPlayerTurnStart` peers whose acquisition order
/// is absent from canonical State.
///
/// SummonNextTurn is deliberately absent: its Osty mutation is disjoint from
/// Hibernate's amount-only decrement, so the two callbacks commute. Every
/// member below can observeably reorder with Hibernate through RNG, Hand,
/// damage, Block, or combat ending.
pub(crate) const HIBERNATE_TURN_START_PEERS: [(PowerId, StepKind); 7] = [
    (PowerId::Loop, StepKind::Loop),
    (PowerId::RollingBoulder, StepKind::RollingBoulder),
    (PowerId::Entropy, StepKind::Entropy),
    (PowerId::ToolsOfTheTrade, StepKind::ToolsOfTheTrade),
    (PowerId::Tyranny, StepKind::Tyranny),
    (PowerId::Inferno, StepKind::Inferno),
    (PowerId::CrimsonMantle, StepKind::CrimsonMantle),
];

/// Map a source-derived row to the unkeyed turn-start power it can install.
///
/// This intentionally keys on the generated program rather than a CardId
/// allowlist. A future carrier of an existing power therefore invalidates the
/// order shortcut automatically instead of silently entering the quotient.
pub(crate) fn hibernate_turn_start_writer(row: &CardRow) -> Option<PowerId> {
    HIBERNATE_TURN_START_PEERS
        .into_iter()
        .find_map(|(power, kind)| {
            row.steps
                .iter()
                .any(|step| step.kind == kind)
                .then_some(power)
        })
}

fn hibernate_program_frost_count(catalog: &Catalog, spec: &CardSpec) -> Result<u8, EngineRefusal> {
    let frost_count = match (spec.identity.id, spec.identity.upgrade) {
        (CardId::Hibernate, 0) => 2,
        (CardId::Hibernate, 1) => 3,
        _ => return Err(EngineRefusal::MalformedArgs("hibernate program")),
    };
    let program = catalog.steps(spec);
    if !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(CardId::Hibernate, spec.identity.upgrade)
            != Some(spec.row)
        || spec.row.rarity != CardRarity::Uncommon
        || spec.row.pool != Some("Defect")
        || spec.row.star_cost != -1
        || spec.row.star_x
        || spec.row.play_condition.is_some()
        || spec.row.on_draw_energy_loss != 0
        || spec.cost != 2
        || !spec.is_skill
        || spec.is_power
        || spec.is_attack
        || spec.is_status
        || spec.is_status_curse
        || spec.strike_tag
        || spec.exhausts
        || spec.targeted
        || spec.target_type != CardTargetType::SelfTarget
        || spec.ethereal
        || spec.innate
        || spec.retain
        || spec.x_cost
        || spec.sly
        || !spec.playable
        || spec.native_unplayable
        || !spec.solo_unplayable
        || spec.selects
        || program.len() != 1 + usize::from(frost_count)
        || !matches!(
            program.first(),
            Some(crate::catalog::CompiledStep {
                kind: StepKind::HibernatePowerExact,
                args,
            }) if catalog.args(*args) == [CompiledArg::I(1)]
        )
        || !program[1..].iter().all(|step| {
            step.kind == StepKind::Channel
                && crate::engine::admission::channel_args(catalog.args(step.args))
                    == Ok((OrbKind::Frost, 1))
        })
    {
        return Err(EngineRefusal::MalformedArgs("hibernate program"));
    }
    Ok(frost_count)
}

/// Whether this immutable catalog member is one of Hibernate's exact L0/L1
/// source programs.
pub(crate) fn hibernate_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    hibernate_program_frost_count(catalog, spec).is_ok()
}

fn exact_hibernate_source(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<u8, EngineRefusal> {
    let frost_count = hibernate_program_frost_count(catalog, spec)?;
    let Some((pile, index)) = crate::engine::play::unique_live_card_location(state, source_uid)?
    else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: 0,
        });
    };
    let source = state.piles.get(pile).as_slice()[index];
    if source.flags & (CARD_FLAG_LEGACY | crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE) != 0
        || catalog.spec(source.atom) != Some(spec)
    {
        return Err(EngineRefusal::MalformedArgs("hibernate physical source"));
    }
    Ok(frost_count)
}

fn require_hibernate_environment(state: &crate::hot::HotState) -> Result<i32, EngineRefusal> {
    let amount = match state.powers.get(PowerId::Hibernate) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value > 0 => slot.value,
        Some(_) => return Err(EngineRefusal::MalformedArgs("hibernate amount")),
    };
    for (peer, _) in HIBERNATE_TURN_START_PEERS {
        let peer_is_live = match peer {
            PowerId::Inferno => {
                state.powers.get(PowerId::Inferno).is_some()
                    || state.powers.get(PowerId::InfernoSelf).is_some()
            }
            PowerId::CrimsonMantle => {
                state.powers.get(PowerId::CrimsonMantle).is_some()
                    || state.powers.get(PowerId::CrimsonSelf).is_some()
            }
            _ => state.powers.get(peer).is_some(),
        };
        if peer_is_live {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Hibernate AfterPlayerTurnStart order",
            ));
        }
    }
    if !crate::engine::damage::player_type_one_listener_order_is_exact(state) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Hibernate power-amount event order",
        ));
    }
    if state.pending.is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(amount)
}

/// v0.111.0 `Hibernate/<OnPlay>` RVA `0x3a50e0` awaits `PowerCmd.Apply<HibernatePower>` at IL_0051.
/// `PowerCmd/<Apply>d__1`1::MoveNext` (`0x3ef988`) returns at `IsEnding` (IL_0025-002a) before the power exists, so the gate is the shared IsEnding projection, not `history.over` (#3515).
fn apply_hibernate_stack(
    state: &mut crate::hot::HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let current = require_hibernate_environment(state)?;
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    let updated = current
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("hibernate"))?;
    state.powers.set(PowerId::Hibernate, SlotWire::Int, updated);
    crate::engine::damage::note_power(events, Subject::Player, PowerId::Hibernate, updated);
    Ok(())
}

/// Authenticate Hibernate's exact source and represented power environment
/// before the shared CardPlay prefix. The existing whole-action checkpoint
/// owns rollback for the later serial-Frost suffix and every replay body.
pub(crate) fn preflight_hibernate_power_exact(
    state: &crate::hot::HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
) -> Result<(), EngineRefusal> {
    exact_hibernate_source(state, catalog, source_uid, spec)?;
    let current = require_hibernate_environment(state)?;
    current
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("hibernate"))?;
    Ok(())
}

/// Validate a live Hibernate listener before the hand draw or any earlier
/// `AfterPlayerTurnStart` callback can publish a prefix.
pub(crate) fn preflight_hibernate_turn_start(
    state: &crate::hot::HotState,
) -> Result<bool, EngineRefusal> {
    if state.history.over {
        return Ok(false);
    }
    let captured = state.powers.get(PowerId::Hibernate).is_some();
    if captured {
        require_hibernate_environment(state)?;
    }
    Ok(captured)
}

/// Execute Hibernate's captured `AfterPlayerTurnStart` decrement.
///
/// Native captures the player-power listener walk, but `PowerCmd::Decrement`
/// enters the ordinary ending-gated `ModifyAmount` command. A listener which
/// was captured while nonterminal therefore still no-ops if an earlier
/// callback ended combat. The public quotient excludes every noncommuting
/// earlier peer; SummonNextTurn remains legal and completes immediately before
/// this amount-only tick.
///
/// `HibernatePower/<AfterPlayerTurnStart>d__6` RVA `0x33c790` awaits
/// `PowerCmd.Decrement` (IL_0033), which forwards to `ModifyAmount`
/// (`<Decrement>d__4` `0x3f025c` IL_0029); `<ModifyAmount>d__6` (`0x3f032c`)
/// returns at `IsEnding` (IL_003a-003f). The shared IsEnding projection, not
/// `history.over` (#3515).
pub(crate) fn tick_hibernate_turn_start(
    state: &mut crate::hot::HotState,
    captured: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !captured || crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    let current = require_hibernate_environment(state)?;
    debug_assert!(current > 0);
    let updated = current - 1;
    state.powers.set(PowerId::Hibernate, SlotWire::Int, updated);
    crate::engine::damage::note_power(events, Subject::Player, PowerId::Hibernate, updated);
    Ok(())
}

/// `hibernate_power_exact` — exact Hibernate duration application.
///
/// Python `_run_steps_inner`'s `hibernate_power_exact` branch calls
/// `_modify_hibernate_amount(+1)`; `_after_turn_start_hand_draw` later
/// decrements one captured live stack. Current v0.111.0 authority (DLL
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Hibernate ctor/constraint/extra-tips/vars/OnPlay-wrapper/upgrade RVAs are
/// `0xe271b`/`0xe2728`/`0xe272b`/`0xe275c`/`0xe276c`/`0xe27bf`; OnPlay's
/// async body RVA `0x3a50e0` applies one power then serially channels two
/// Frost at L0 or three at L1. HibernatePower's Type/StackType/hook-wrapper/
/// ctor RVAs are `0xa392b`/`0xa392e`/`0xa3954`/`0xa399f`.
/// `HibernatePower/<AfterPlayerTurnStart>d__6::MoveNext` RVA `0x33c790`
/// awaits exactly one decrement after the hand draw; generic ModifyAmount
/// RVA `0x3f032c` checks combat ending before the amount write. The `Slots`
/// carrier keeps this duration O(live fight powers); the existing CardPlay
/// checkpoint makes a later full-queue Evoke refusal restore the whole action.
pub(crate) fn hibernate_power_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if ctx.args != [CompiledArg::I(1)]
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("hibernate_power_exact"));
    }
    exact_hibernate_source(ctx.state, ctx.catalog, ctx.source_uid, ctx.spec)?;
    apply_hibernate_stack(ctx.state, ctx.events)
}

fn ignition_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    if !crate::engine::play::body_enchantment_is_exact(spec)
        || !matches!(
            (spec.identity.id, spec.identity.upgrade),
            (CardId::Ignition, 0 | 1)
        )
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
        || !matches!(
            catalog.steps(spec),
            [crate::catalog::CompiledStep {
                kind: StepKind::IgnitionExact,
                args,
            }] if catalog.args(*args).is_empty()
        )
    {
        return false;
    }

    // The private foundation claims the complete L0/L1 carrier family, so a
    // fight catalog missing the replay/upgrade peer is not sufficient proof.
    (0..=1).all(|upgrade| {
        let identity = CardIdentity {
            id: CardId::Ignition,
            upgrade,
            enchantment: None,
        };
        catalog
            .atom(&identity)
            .and_then(|atom| catalog.spec(atom))
            .is_some_and(|peer| {
                peer.identity == identity
                    && crate::content_tables::card_row(CardId::Ignition, upgrade) == Some(peer.row)
                    && matches!(
                        catalog.steps(peer),
                        [crate::catalog::CompiledStep {
                            kind: StepKind::IgnitionExact,
                            args,
                        }] if catalog.args(*args).is_empty()
                    )
            })
    })
}

fn ignition_owner_context_is_exact(ctx: &StepCtx<'_>) -> bool {
    if !ctx.args.is_empty()
        || ctx.target != Some(0)
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.state.history.over
        || !ctx.state.player_side_active
        || ctx.state.hp <= 0
        || !ctx.state.exact_piles
        || !ignition_program_is_exact(ctx.catalog, ctx.spec)
        || !crate::engine::play::apotheosis_body_context_is_exact(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
        )
    {
        return false;
    }

    let Some((pile, index)) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)
            .ok()
            .flatten()
    else {
        return false;
    };
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if pile != PileId::Play
        || source.flags != 0
        || !ctx.state.card_states.get(source.uid).is_vacant()
        || ctx.catalog.spec(source.atom) != Some(ctx.spec)
    {
        return false;
    }

    // Ignition is MultiplayerOnly. The accepted owner-target branch keeps
    // one exact, living remote Player solely as multiplayer provenance; its
    // complete default quotient proves that no represented remote hook can
    // observe the owner's Channel command.
    let expected_ally = MultiplayerAllyState {
        key: 1,
        alive: true,
        ..MultiplayerAllyState::default()
    };
    if ctx.state.multiplayer_ally_key != 1 || ctx.state.fanouts.multiplayer_ally() != &expected_ally
    {
        return false;
    }

    // Current native AfterOrbChanneled has only Metronome, an unrepresented
    // relic (Hook RVA 0x10421c; Metronome RVA 0x96da4 / body 0x32a960).
    // Close every represented orb modifier/listener carrier anyway:
    // Plasma itself bypasses Focus, but vacant power slots and private orb
    // counters make that skipped hook projection structural rather than an
    // assumption about a conveniently inert value.
    //
    // `lightning_channeled` (Voltaic's tracked-or-untracked count, `#3397`)
    // is deliberately left unconstrained here rather than pinned to the
    // untracked `-1`. `OrbCmd/<Channel>d__3::MoveNext` (RVA `0x3ed69c`)
    // records `CombatHistory::OrbChanneled(combatState, orb)` at `IL_021a`
    // for every successfully enqueued orb regardless of kind (gated only by
    // the `TryEnqueue` bool at `IL_01ff`); it never branches on `orb`'s
    // type. Voltaic's canonical-var predicate,
    // `<>c__DisplayClass5_0::<get_CanonicalVars>b__1` (RVA `0x3c6ce8`),
    // narrows that shared history to entries whose `Orb` `is LightningOrb`
    // (`IL_0019`-`IL_0027`). Ignition's own `<OnPlay>d__7::MoveNext` (RVA
    // `0x3a69b8`) awaits `Channel<PlasmaOrb>` at `IL_00c9`/`IL_00ce`
    // (`MethodSpec` generic argument resolves to
    // `MegaCrit.Sts2.Core.Models.Orbs.PlasmaOrb`), so the history entry this
    // Channel appends is a `PlasmaOrb` entry: it can never satisfy Voltaic's
    // `LightningOrb` predicate. Whatever `lightning_channeled` held before
    // Ignition — untracked (`-1`) or a tracked non-negative count — it holds
    // the identical value after, because [`crate::engine::orbs::channel`]
    // only advances the field when `kind == OrbKind::Lightning`, which this
    // call site never is. The private probe below clones `ctx.state`,
    // channels through the same function, and copies the clone back
    // wholesale, so the untouched field carries over unchanged either way.
    let orbs = &ctx.state.orbs;
    if !ctx.state.powers.is_empty()
        || orbs.temp_focus() != 0
        || orbs.orbit_energy_spent() != 0
        || orbs.orbit_trigger_count() != 0
        || orbs.next_random_orb_progress_uid() != 0
        || orbs.next_consuming_shadow_side_end_auth_uid() != 0
        || !orbs.reset_order().is_empty()
        || !matches!(orbs.base_slots(), 0 | 3)
        || orbs.slots() > MAX_ORB_SLOTS
    {
        return false;
    }

    // Excluding a full positive-capacity queue proves Channel cannot enter
    // EvokeNext. Zero/zero instead takes the native one-slot bootstrap before
    // enqueueing. Both accepted shapes therefore append exactly one Plasma,
    // consume no RNG, and have no fallible delegated suffix.
    (orbs.base_slots() == 0 && orbs.slots() == 0 && orbs.as_slice().is_empty())
        || (orbs.slots() > 0 && orbs.as_slice().len() < usize::from(orbs.slots()))
}

/// Private/test-only exact owner-target body for Ignition.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Ignition` constructor/MultiplayerConstraint/keywords/OnPlay/upgrade RVAs
/// are `0xe2fff`, `0xe300c`, `0xe300f`, `0xe303c`, and `0xe308f`.
/// `Ignition/<OnPlay>d__7::MoveNext` RVA `0x3a69b8` validates the non-null
/// target, awaits presentation-only Cast, re-reads `CardPlay.Target.Player`,
/// then awaits one `Channel<PlasmaOrb>`. The generated L0/L1 rows are the
/// sole empty-argument `IgnitionExact` step; L0 Exhausts and L1 does not.
///
/// Remote-target Ignition remains outside the Rust quotient because
/// [`MultiplayerAllyState`] intentionally has no orb queue. This exact branch
/// instead authenticates target key zero, a canonical live teammate, and one
/// default-state exact physical source in Play, then clone-rehearses the
/// complete nonfull owner Channel before publishing it. A terminal entry is
/// refused: this single-step body has no preceding effect that can end combat,
/// so Channel's native terminal no-op is unreachable from this private seam.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn ignition_owner_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ignition_owner_context_is_exact(ctx) {
        return Err(EngineRefusal::MalformedArgs("ignition owner foundation"));
    }
    let mut probe = ctx.state.clone();
    let mut emitted = Vec::new();
    crate::engine::orbs::channel(&mut probe, ctx.catalog, OrbKind::Plasma, &mut emitted)?;
    *ctx.state = probe;
    ctx.events.extend(emitted);
    Ok(())
}

/// `ignition_exact` — not modeled. **Escalated (#1322).**
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) resolves `_resolve_ally_target_key`, then
/// calls `_channel_plasma_to_player`.
///
/// The private owner-target foundation above re-derives the now-represented
/// local branch. Public integration remains escalated: native AnyAlly also
/// permits the live remote Player, while [`MultiplayerAllyState`] still owns
/// no orb queue or orb-hook projection. Publishing only key zero would remove
/// a legal target; aliasing key one to the owner's queue would mutate the
/// wrong Player. The public card therefore stays typed-refused until the
/// remote queue and outer play transaction land together.
pub(crate) fn ignition_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::IgnitionExact))
}

/// Chaos's exact authenticated GetRandomOrb -> Channel loop.
///
/// Python `_run_steps_inner` (frozen, deleted #2827) authenticates the exact carrier;
/// `_begin_random_orb_loop` owns the serial random-Channel lifecycle.
pub(crate) fn random_orb_loop_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [program] = ctx.catalog.steps(ctx.spec) else {
        return Err(EngineRefusal::MalformedArgs(
            "random_orb_loop_exact program",
        ));
    };
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Chaos
        || ctx.spec.identity.upgrade > 1
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
        || program.kind != StepKind::RandomOrbLoopExact
        || !ctx.catalog.args(program.args).is_empty()
    {
        return Err(EngineRefusal::MalformedArgs("random_orb_loop_exact"));
    }
    let (pile, index) = crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
        .ok_or(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        })?;
    let source = ctx.state.piles.get(pile).as_slice()[index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "random_orb_loop_exact physical source",
        ));
    }
    let mut next = ctx.state.clone();
    let mut emitted = Vec::new();
    crate::engine::orbs::channel_random_loop(
        &mut next,
        ctx.catalog,
        u32::from(1 + ctx.spec.identity.upgrade),
        &mut emitted,
    )?;
    *ctx.state = next;
    ctx.events.extend(emitted);
    Ok(())
}

/// Scrape's attack, exact returned-draw filter, and ordered discard suffix.
///
/// Native authority: current v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Scrape/<>c::<OnPlay>b__3_0` at RVA `0x3b896a` calls
/// `EnergyCost.GetWithModifiers(-1)` and returns true for a nonzero result;
/// a zero result falls through to `CostsX`. `Scrape/<OnPlay>d__3::MoveNext`
/// at RVA `0x3b8988` filters the complete awaited Draw with that predicate
/// before its one `CardCmd::Discard` command.
///
/// The frozen Python `_run_steps_inner` (deleted #2827) / `_scrape_exact` /
/// `_scrape_after_draw` path is retained only as a compatibility
/// counterpart. The returned Draw owns the physical identities; Scrape
/// resolves each card's then-live energy cost in that order and discards
/// exactly nonzero-cost or X-cost results. The live piles are searched as the
/// frozen counterpart does: Hand and Play use the live cost pipeline, while
/// the other piles use the same result under the admitted surface (the omitted
/// Free* powers and pile-gated Scarf are both refused). Selected references retain
/// occurrence order, including repeated UIDs. The one zero-draw
/// `DiscardAndDraw` transaction moves each current object separately before
/// its history and discard callbacks. A DUPE Strike that Hellraiser
/// auto-played and removed during the Draw stays in the returned list: it is
/// priced from its local rows only (null `CombatState`) and, when selected,
/// counts as discarded without moving (`discard_scrape_occurrences`, #3136).
///
///
/// `Scrape/<OnPlay>d__3` RVA `0x3b8988` awaits its attack (IL_0084), then
/// `CardPileCmd.Draw` (IL_0104), then `CardCmd.Discard` of the drawn non-zero
/// costs (IL_0193). `<DrawInternal>d__21` (`0x3e3a70` IL_0029-003b) and
/// `<DiscardAndDraw>d__4` (`0x3e0274` IL_002e-0035) both return at
/// `IsOverOrEnding`, so once the attack leaves the combat ending the whole tail
/// is a no-op: the shared IsOverOrEnding projection, not `history.over`
/// (#3515).
pub(crate) fn scrape_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(damage), CompiledArg::I(cards)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("scrape_exact"));
    };
    let expected = match ctx.spec.identity.upgrade {
        0 => (7, 4),
        1 => (10, 5),
        _ => return Err(EngineRefusal::MalformedArgs("scrape_exact")),
    };
    if ctx.spec.identity.id != CardId::Scrape || (*damage, *cards) != expected {
        return Err(EngineRefusal::MalformedArgs("scrape_exact"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        *damage,
        1,
        ctx.events,
    )?;
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    normalize_card_identities(ctx.state)?;
    let cards: usize = (*cards)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("scrape_exact"))?;
    let drawn = if matches!(
        ctx.state.frames.top(),
        Some(crate::frame::Frame::CardPlay { .. })
    ) {
        match crate::engine::play::draw_cardplay_owned_result(ctx, cards)? {
            CardDrawResult::Complete(drawn) => {
                drawn.into_iter().map(crate::hot::DrawEntry::card).collect()
            }
            CardDrawResult::Suspended => return Ok(()),
        }
    } else {
        let mut drawn = Vec::<HotCard>::new();
        draw_cards_into(
            ctx.state,
            ctx.catalog,
            cards,
            DrawSource::Command,
            ctx.events,
            &mut drawn,
        )?;
        drawn
    };
    apply_scrape_after_draw(ctx.state, ctx.catalog, &drawn, ctx.events)
}

/// The Discard is `CardCmd/<DiscardAndDraw>d__4` (`0x3e0274`), which returns at
/// `IsOverOrEnding` (IL_002e-0035): a draw listener that leaves the combat
/// ending discards nothing (#3515).
fn apply_scrape_after_draw(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    drawn: &[HotCard],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }

    // Scrape0x3b8988 passes a lazy Where into Discard; DiscardAndDraw
    // 0x3e0274 materializes it before any move or callback. Preserve all
    // returned occurrences while reading each original's current live cost.
    // A returned object Hellraiser auto-played and removed (DUPE) is priced
    // from its retained local rows alone; see
    // `removed_draw_object_energy_cost` for the null-CombatState IL (#3136).
    let mut selected = Vec::new();
    for returned in drawn {
        let live = crate::engine::draw::drawn_object_card(state, returned.uid)?;
        let spec = catalog
            .spec(live.atom)
            .ok_or(EngineRefusal::UnknownAtom(live.atom))?;
        let cost = match crate::engine::draw::removed_draw_object_energy_cost(
            state,
            catalog,
            returned.uid,
        )? {
            Some(removed) => removed,
            None => resolved_energy_cost(state, live, spec),
        };
        if cost != 0 || spec.x_cost {
            selected.push(live);
        }
    }
    crate::engine::draw::discard_scrape_occurrences(state, catalog, &selected, events)
}

pub(crate) fn resume_scrape_after_draw(
    state: &mut crate::hot::HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    drawn: &[crate::hot::DrawEntry],
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !crate::engine::play::cardplay_owned_draw_tail_step_is_exact(spec, catalog, 0) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let drawn = drawn
        .iter()
        .copied()
        .map(crate::hot::DrawEntry::card)
        .collect::<Vec<_>>();
    apply_scrape_after_draw(state, catalog, &drawn, events)
}

/// The frozen fully-unlocked owner-Power table, retained for the witnesses.
#[cfg(test)]
fn owner_power_pool(owner: crate::catalog::RewardPool) -> &'static [CardId] {
    use crate::catalog::RewardPool;
    let owner_name = match owner {
        RewardPool::Defect => "Defect",
        RewardPool::Ironclad => "Ironclad",
        RewardPool::Necrobinder => "Necrobinder",
        RewardPool::Regent => "Regent",
        RewardPool::Silent => "Silent",
    };
    crate::content_tables::ABUNDANCE_POWER_POOLS_V1101
        .iter()
        .find_map(|(name, pool)| (*name == owner_name).then_some(*pool))
        .expect("every RewardPool has a current-build Power pool")
}

/// White Noise's exact owner-Power generation transaction.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) calls `_apply_white_noise_exact`:
/// consume one complete shuffle of the exact active owner's fully-unlocked
/// solo Power pool, take the first fresh L0 identity, apply
/// `SetToFreeThisTurn`, then begin one singular generated-card transaction
/// into Hand/Bottom (or Discard/Bottom at cap).
///
/// Current v0.111.0 native authority: installed and archived `sts2.dll`
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `WhiteNoise/<OnPlay>d__3::MoveNext` RVA `0x3c74ac` reads the exact owner,
/// Character CardPool, UnlockState, and multiplayer constraint at IL
/// `0x009e-0x00cf`; predicate RVA `0x3c749e` keeps exactly CardType Power;
/// IL `0x00f3-0x0118` requests one `GetDistinctForCombat` result from
/// CombatCardGeneration, and IL `0x0119-0x0185` makes that exact card free and
/// awaits one Hand/Bottom generated add. Constructor/keywords/upgrade RVAs
/// `0xf0328/0xf0335/0xf0383` pin costs 1/0 and Exhaust while the result stays L0.
pub(crate) fn white_noise_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !matches!(
        (
            ctx.spec.identity.id,
            ctx.spec.identity.upgrade,
            ctx.args,
            ctx.target,
            ctx.selection,
            ctx.x_value,
        ),
        (CardId::WhiteNoise, 0 | 1, [], None, None, 0)
    ) {
        return Err(EngineRefusal::MalformedArgs("white_noise_exact"));
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
    if source_pile == PileId::Exhaust
        && !crate::engine::play::active_play_stack_contains_uid(ctx.source_uid)
    {
        return Err(EngineRefusal::MalformedArgs(
            "white noise generation provenance",
        ));
    }
    let source = &ctx.state.piles.get(source_pile).as_slice()[source_index];
    // #2560/#2946: the OWNER's Power pool under the recorded profile
    // (`Catalog::owner_type_generation_pool`). The frozen
    // `ABUNDANCE_POWER_POOLS_V1101` row is this derivation at the
    // fully-unlocked profile (`one_type_owner_pools_reproduce_the_frozen_constants`).
    if ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || !crate::engine::cards::owner_pool_profile_provenance_is_exact(ctx.state, ctx.catalog)
    {
        return Err(EngineRefusal::MalformedArgs(
            "white noise generation provenance",
        ));
    }
    let owner = ctx
        .state
        .reward_card_pool
        .ok_or(EngineRefusal::MalformedArgs(
            "white noise generation provenance",
        ))?;
    let pool = ctx
        .catalog
        .owner_type_generation_pool(owner, crate::content_tables::CardType::Power);
    if pool.is_empty() {
        return Err(EngineRefusal::MalformedArgs(
            "white noise generation provenance",
        ));
    }

    fn apply(ctx: &mut StepCtx<'_>, pool: &[CardId]) -> Result<(), EngineRefusal> {
        let shuffled = shuffle_generation_slice(ctx.state, pool)?;
        inject_generated_free_this_turn_bottom(
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

    // A selected leaf or generated listener can refuse after RNG advances.
    // Rehearse the complete synchronous body so direct callers retain the
    // public action's all-or-nothing boundary. Every reached reader/callback
    // is carried by HotState plus the immutable catalog.
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
    use crate::catalog::RewardPool;
    use crate::catalog::{CardIdentity, Catalog, CatalogBuilder};
    use crate::content_tables::CARD_ROWS;
    use crate::engine::play::{autoplay_collected_cards, autoplay_draw_top, play_card};
    use crate::engine::{Action, Event, SelectionRef, apply_action_into};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_RINGING, HopperDeckRow, HopperDeckState,
        HotMonster, HotOrb, HotState, LocalCostExpiration, LocalCostModifier,
        LocalCostModifierKind, MultiplayerAllyState, RngStream, RngStreamState,
    };
    use crate::ids::{MonsterKind, PowerId};
    use crate::powers::SlotWire;
    use crate::rng::Xoshiro256StarStar;

    fn run(
        kind: StepKind,
        identity: CardIdentity,
        args: &[CompiledArg],
        target: Option<usize>,
        state: &mut HotState,
    ) -> Result<(), EngineRefusal> {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        run_with_catalog(kind, identity, args, target, state, &catalog)
    }

    fn run_with_catalog(
        kind: StepKind,
        identity: CardIdentity,
        args: &[CompiledArg],
        target: Option<usize>,
        state: &mut HotState,
        catalog: &Catalog,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
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
        crate::steps::apply_step(kind, &mut ctx)
    }

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn compact_parts(upgrade: u8) -> (HotState, Catalog, HotCard) {
        let compact = identity(CardId::Compact, upgrade);
        let mut builder = CatalogBuilder::new();
        let compact_atom = builder.intern(compact).unwrap();
        for identity in [
            identity(CardId::DefendDefect, 0),
            identity(CardId::Burn, 0),
            identity(CardId::StrikeDefect, 0),
            identity(CardId::Wound, 0),
        ] {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: compact_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 6;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let cards = [
            (CardId::DefendDefect, 2),
            (CardId::Burn, 3),
            (CardId::StrikeDefect, 4),
            (CardId::Wound, 5),
        ]
        .map(|(id, uid)| HotCard {
            uid,
            atom: catalog.atom(&identity(id, 0)).unwrap(),
            flags: 0,
        });
        state.piles.get_mut(PileId::Hand).make_mut().extend(cards);
        (state, catalog, source)
    }

    fn ignition_parts(upgrade: u8) -> (HotState, Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atoms = (0..=1)
            .map(|level| builder.intern(identity(CardId::Ignition, level)).unwrap())
            .collect::<Vec<_>>();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom: atoms[usize::from(upgrade)],
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 80;
        state.exact_piles = true;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            alive: true,
            ..MultiplayerAllyState::default()
        });
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source)
    }

    fn call_ignition_owner(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        target: Option<usize>,
        args: &[CompiledArg],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        ignition_owner_foundation_exact(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target,
            selection: None,
            x_value: 0,
            args,
            events,
        })
    }

    fn run_ignition_owner(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        target: Option<usize>,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        crate::engine::play::with_test_active_play(source.uid, || {
            call_ignition_owner(state, catalog, source, target, &[], events)
        })
    }

    fn assert_ignition_refuses_atomically(
        mut state: HotState,
        catalog: &Catalog,
        source: HotCard,
        target: Option<usize>,
    ) {
        let before = state.clone();
        let mut events = vec![Event::PlayerBlockGained {
            amount: 2,
            block: 3,
        }];
        let before_events = events.clone();
        assert!(
            run_ignition_owner(&mut state, catalog, source, target, &mut events).is_err(),
            "the malformed Ignition foundation state was accepted"
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn ignition_owner_foundation_bootstraps_or_appends_one_plasma_for_both_levels() {
        for upgrade in 0..=1 {
            let (mut bootstrap, catalog, source) = ignition_parts(upgrade);
            // Channel checks IsEnding, which is trivially true for a
            // monsterless state; Ignition is played in live combat.
            bootstrap
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            let entering_rng = bootstrap.rng.clone();
            let mut events = vec![Event::PlayerBlockGained {
                amount: 2,
                block: 3,
            }];
            run_ignition_owner(&mut bootstrap, &catalog, source, Some(0), &mut events).unwrap();
            assert_eq!(bootstrap.orbs.base_slots(), 0);
            assert_eq!(bootstrap.orbs.slots(), 1);
            assert_eq!(
                bootstrap.orbs.as_slice(),
                [HotOrb::from_parts(OrbKind::Plasma, None).unwrap()].as_slice()
            );
            assert_eq!(bootstrap.rng, entering_rng);
            assert_eq!(
                events,
                [Event::PlayerBlockGained {
                    amount: 2,
                    block: 3,
                }]
            );

            let (mut nonfull, catalog, source) = ignition_parts(upgrade);
            // Same live-combat requirement as above for the Channel suffix.
            nonfull
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            nonfull.orbs.set_base_slots(3);
            nonfull.orbs.set_slots(3);
            nonfull.orbs.set_orbs(vec![
                HotOrb::from_parts(OrbKind::Frost, None).unwrap(),
                HotOrb::from_parts(OrbKind::Dark, Some(11)).unwrap(),
            ]);
            let entering_rng = nonfull.rng.clone();
            run_ignition_owner(&mut nonfull, &catalog, source, Some(0), &mut Vec::new()).unwrap();
            assert_eq!(nonfull.orbs.slots(), 3);
            assert_eq!(
                nonfull
                    .orbs
                    .as_slice()
                    .iter()
                    .map(|orb| orb.kind())
                    .collect::<Vec<_>>(),
                vec![OrbKind::Frost, OrbKind::Dark, OrbKind::Plasma]
            );
            assert_eq!(nonfull.rng, entering_rng);
        }
    }

    /// `#3397`: a Voltaic-tracked `lightning_channeled` count is accepted and
    /// left untouched, because Ignition channels a `PlasmaOrb`
    /// (`<OnPlay>d__7::MoveNext` RVA `0x3a69b8`, IL_00c9 `Channel<PlasmaOrb>`)
    /// and only a `LightningOrb` history entry advances Voltaic's count
    /// (predicate RVA `0x3c6ce8`). An untracked (`-1`) count still admits,
    /// covering the sibling row.
    #[test]
    fn ignition_owner_foundation_leaves_a_tracked_lightning_count_unchanged() {
        for tracked in [0, 1, 7] {
            let (mut state, catalog, source) = ignition_parts(0);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.orbs.set_lightning_channeled(tracked);
            run_ignition_owner(&mut state, &catalog, source, Some(0), &mut Vec::new()).unwrap();
            assert_eq!(state.orbs.lightning_channeled(), tracked);
            assert_eq!(
                state.orbs.as_slice(),
                [HotOrb::from_parts(OrbKind::Plasma, None).unwrap()].as_slice()
            );
        }

        // The untracked sentinel (the pre-#3397 accepted row) still admits
        // and still stays untracked.
        let (mut untracked, catalog, source) = ignition_parts(0);
        untracked
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        assert_eq!(untracked.orbs.lightning_channeled(), -1);
        run_ignition_owner(&mut untracked, &catalog, source, Some(0), &mut Vec::new()).unwrap();
        assert_eq!(untracked.orbs.lightning_channeled(), -1);
    }

    #[test]
    fn ignition_owner_foundation_requires_current_physical_source_and_both_rows() {
        let (mut state, catalog, source) = ignition_parts(0);
        let before = state.clone();
        let mut events = vec![Event::PlayerBlockGained {
            amount: 2,
            block: 3,
        }];
        let before_events = events.clone();
        assert!(
            call_ignition_owner(&mut state, &catalog, source, Some(0), &[], &mut events).is_err()
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);

        crate::engine::play::with_test_active_play(99, || {
            assert!(
                call_ignition_owner(&mut state, &catalog, source, Some(0), &[], &mut events,)
                    .is_err()
            );
        });
        assert_eq!(state, before, "a noncurrent parent source changed state");
        assert_eq!(events, before_events);

        crate::engine::play::with_test_active_play(source.uid, || {
            assert!(
                call_ignition_owner(
                    &mut state,
                    &catalog,
                    source,
                    Some(0),
                    &[CompiledArg::I(1)],
                    &mut events,
                )
                .is_err()
            );
        });
        assert_eq!(state, before, "a forged operand changed state");
        assert_eq!(events, before_events);

        let mut wrong_pile = before.clone();
        let moved = wrong_pile.piles.get_mut(PileId::Play).make_mut().remove(0);
        wrong_pile
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(moved);
        assert_ignition_refuses_atomically(wrong_pile, &catalog, source, Some(0));

        let mut duplicate = before.clone();
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        assert_ignition_refuses_atomically(duplicate, &catalog, source, Some(0));

        let mut forged_flags = before.clone();
        let (pile, index) =
            crate::engine::play::unique_live_card_location(&forged_flags, source.uid)
                .unwrap()
                .unwrap();
        forged_flags.piles.get_mut(pile).make_mut()[index].flags |=
            CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        assert_ignition_refuses_atomically(forged_flags, &catalog, source, Some(0));

        let mut forged_instance = before.clone();
        forged_instance.card_states.append_local_cost_modifier(
            source.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            },
        );
        assert_ignition_refuses_atomically(forged_instance, &catalog, source, Some(0));

        let mut nonexact = before.clone();
        nonexact.exact_piles = false;
        assert_ignition_refuses_atomically(nonexact, &catalog, source, Some(0));

        let mut incomplete = CatalogBuilder::new();
        let atom = incomplete.intern(identity(CardId::Ignition, 0)).unwrap();
        let incomplete = incomplete.build();
        let mut missing_peer = before;
        missing_peer.piles.get_mut(PileId::Play).make_mut()[0].atom = atom;
        let incomplete_source = HotCard { atom, ..source };
        assert_ignition_refuses_atomically(missing_peer, &incomplete, incomplete_source, Some(0));
    }

    #[test]
    fn ignition_owner_foundation_closes_target_party_terminal_and_orb_state_atomically() {
        let (state, catalog, source) = ignition_parts(1);
        assert_ignition_refuses_atomically(state.clone(), &catalog, source, None);
        assert_ignition_refuses_atomically(state.clone(), &catalog, source, Some(1));

        let mut solo = state.clone();
        solo.multiplayer_ally_key = 0;
        assert_ignition_refuses_atomically(solo, &catalog, source, Some(0));

        let mut noncanonical_ally = state.clone();
        noncanonical_ally.fanouts.multiplayer_ally_mut().block = 1;
        assert_ignition_refuses_atomically(noncanonical_ally, &catalog, source, Some(0));

        let mut ended = state.clone();
        ended.history.over = true;
        assert_ignition_refuses_atomically(ended, &catalog, source, Some(0));

        let mut wrong_wire = state.clone();
        wrong_wire.powers.set(PowerId::Focus, SlotWire::Bool, 1);
        assert_ignition_refuses_atomically(wrong_wire, &catalog, source, Some(0));

        let mut temp_focus = state.clone();
        temp_focus.orbs.set_temp_focus(1);
        assert_ignition_refuses_atomically(temp_focus, &catalog, source, Some(0));

        let mut bad_base = state.clone();
        bad_base.orbs.set_base_slots(2);
        bad_base.orbs.set_slots(3);
        assert_ignition_refuses_atomically(bad_base, &catalog, source, Some(0));

        let mut bad_cap = state.clone();
        bad_cap.orbs.set_base_slots(3);
        bad_cap.orbs.set_slots(MAX_ORB_SLOTS + 1);
        assert_ignition_refuses_atomically(bad_cap, &catalog, source, Some(0));

        let mut full = state;
        full.orbs.set_base_slots(3);
        full.orbs.set_slots(1);
        full.orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Plasma, None).unwrap()]);
        full.energy = i16::MAX;
        assert_ignition_refuses_atomically(full, &catalog, source, Some(0));
    }

    #[test]
    fn ignition_private_rows_remain_the_exact_two_level_family() {
        let rows = crate::content_tables::card_rows(CardId::Ignition);
        assert_eq!(rows.len(), 2);
        for (upgrade, row) in rows.iter().enumerate() {
            assert_eq!(row.upgrade, upgrade as u8);
            assert_eq!(row.cost, 1);
            assert_eq!(row.target_type, "AnyAlly");
            assert!(row.playable && row.targeted && row.is_skill && !row.is_power);
            assert_eq!(row.exhausts, upgrade == 0);
            assert_eq!(
                row.steps,
                [crate::content_tables::Step {
                    kind: StepKind::IgnitionExact,
                    args: &[],
                }]
            );
        }
        assert!(!IMPLEMENTED.contains(&StepKind::IgnitionExact));
        assert!(
            !crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::IgnitionExact)
        );
    }

    fn run_compact(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        args: &[CompiledArg],
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        compact_exact(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args,
            events,
        })
    }

    #[test]
    fn compact_bulk_replaces_statuses_at_exact_indexes_with_fresh_fuel() {
        for upgrade in 0..=1 {
            let (mut state, catalog, source) = compact_parts(upgrade);
            state.set_ringing(true);
            let entering_rng = state.rng.clone();
            let mut events = Vec::new();
            run_compact(
                &mut state,
                &catalog,
                source,
                &[
                    CompiledArg::I(6 + i64::from(upgrade)),
                    CompiledArg::I(i64::from(upgrade)),
                ],
                &mut events,
            )
            .unwrap();

            assert_eq!(state.block, 6 + i32::from(upgrade));
            assert_eq!(state.next_card_uid, 8);
            assert_eq!(state.history.owner_generated_cards_combat, 2);
            assert_eq!(state.next_generated_hook_uid, 2);
            assert!(state.exact_piles);
            assert_eq!(state.rng, entering_rng, "Compact itself consumes no RNG");
            assert_eq!(
                state.piles.get(PileId::Play).as_slice(),
                [source].as_slice(),
                "the active Skill is not a Discard Status candidate"
            );
            assert!(
                state.piles.get(PileId::Discard).is_empty(),
                "Compact reads Hand (GetPile(2)); nothing reaches Discard"
            );
            assert!(
                events
                    .iter()
                    .all(|event| matches!(event, Event::PlayerBlockGained { .. }))
            );
            let hand = state.piles.get(PileId::Hand).as_slice();
            assert_eq!(
                hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
                vec![2, 6, 7, 4],
                "Burn records index 1; Wound records 2 once Burn has left (#3199)"
            );
            assert_eq!(
                hand.iter()
                    .map(|card| catalog.spec(card.atom).unwrap().identity)
                    .collect::<Vec<_>>(),
                vec![
                    identity(CardId::DefendDefect, 0),
                    identity(CardId::Fuel, upgrade),
                    identity(CardId::Fuel, upgrade),
                    identity(CardId::StrikeDefect, 0),
                ]
            );
            assert!(
                [hand[1], hand[2]]
                    .into_iter()
                    .all(|card| card.flags & CARD_FLAG_RINGING != 0),
                "fresh Fuel traverses the physical-entry listener"
            );
            assert!(PileId::ALL.into_iter().all(|pile| {
                state
                    .piles
                    .get(pile)
                    .as_slice()
                    .iter()
                    .all(|card| !matches!(card.uid, 3 | 5))
            }));
        }
    }

    /// #3052 witness: `GetPile(2)` is Hand. Beckon, Slimed and Dazed in Hand
    /// become Fuel (at the #3199 sequential indexes); the same Statuses in Discard (and Draw) are
    /// untouched. A full ten-card Hand never overflows, because the plural
    /// Transform re-inserts with raw `AddInternal` after removing every
    /// original.
    #[test]
    fn compact_transforms_hand_statuses_only_and_a_full_hand_never_overflows() {
        let compact = identity(CardId::Compact, 0);
        let mut builder = CatalogBuilder::new();
        let compact_atom = builder.intern(compact).unwrap();
        for id in [
            CardId::Beckon,
            CardId::Slimed,
            CardId::Dazed,
            CardId::StrikeDefect,
            CardId::DefendDefect,
        ] {
            builder.intern(identity(id, 0)).unwrap();
        }
        let catalog = builder.build();
        let card = |id: CardId, uid: u32| HotCard {
            uid,
            atom: catalog.atom(&identity(id, 0)).unwrap(),
            flags: 0,
        };
        let source = HotCard {
            uid: 1,
            atom: compact_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let hand_ids = [
            CardId::StrikeDefect,
            CardId::Beckon,
            CardId::DefendDefect,
            CardId::Slimed,
            CardId::Dazed,
            CardId::StrikeDefect,
            CardId::DefendDefect,
            CardId::Dazed,
            CardId::StrikeDefect,
            CardId::DefendDefect,
        ];
        let hand = hand_ids
            .iter()
            .zip(10..)
            .map(|(id, uid)| card(*id, uid))
            .collect::<Vec<_>>();
        assert_eq!(hand.len(), crate::engine::draw::MAX_CARDS_IN_HAND);
        state.piles.get_mut(PileId::Hand).make_mut().extend(hand);
        let discard = vec![
            card(CardId::Beckon, 30),
            card(CardId::Slimed, 31),
            card(CardId::Dazed, 32),
        ];
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(discard.iter().copied());
        let draw = vec![card(CardId::Slimed, 40)];
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend(draw.iter().copied());

        run_compact(
            &mut state,
            &catalog,
            source,
            &[CompiledArg::I(6), CompiledArg::I(0)],
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            discard.as_slice()
        );
        assert_eq!(state.piles.get(PileId::Draw).as_slice(), draw.as_slice());
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), crate::engine::draw::MAX_CARDS_IN_HAND);
        assert_eq!(
            hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
            vec![10, 100, 102, 101, 103, 12, 15, 16, 18, 19],
            "each Fuel lands at its Status's index once the earlier Statuses left \
             (1, 2, 2, 4; #3199); fresh uids in candidate order"
        );
        let fuel = identity(CardId::Fuel, 0);
        assert_eq!(
            hand.iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect::<Vec<_>>(),
            vec![
                CardId::StrikeDefect,
                CardId::Fuel,
                CardId::Fuel,
                CardId::Fuel,
                CardId::Fuel,
                CardId::DefendDefect,
                CardId::StrikeDefect,
                CardId::DefendDefect,
                CardId::StrikeDefect,
                CardId::DefendDefect,
            ]
        );
        assert!(
            [hand[1], hand[2], hand[3], hand[4]]
                .into_iter()
                .all(|card| catalog.spec(card.atom).unwrap().identity == fuel)
        );
        assert_eq!(state.next_card_uid, 104);
        assert_eq!(state.history.owner_generated_cards_combat, 4);
        assert_eq!(state.block, 6);
    }

    /// Compact over `hand_ids` (uids from 10, fresh uids from 100); returns
    /// the Hand as (uid, id) after the play.
    fn compact_hand_after(hand_ids: &[CardId]) -> Vec<(u32, CardId)> {
        let mut builder = CatalogBuilder::new();
        let compact_atom = builder.intern(identity(CardId::Compact, 0)).unwrap();
        for id in hand_ids {
            builder.intern(identity(*id, 0)).unwrap();
        }
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: compact_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let hand = hand_ids
            .iter()
            .zip(10..)
            .map(|(id, uid)| HotCard {
                uid,
                atom: catalog.atom(&identity(*id, 0)).unwrap(),
                flags: 0,
            })
            .collect::<Vec<_>>();
        state.piles.get_mut(PileId::Hand).make_mut().extend(hand);
        run_compact(
            &mut state,
            &catalog,
            source,
            &[CompiledArg::I(6), CompiledArg::I(0)],
            &mut Vec::new(),
        )
        .unwrap();
        state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| (card.uid, catalog.spec(card.atom).unwrap().identity.id))
            .collect()
    }

    /// #3199 witnesses, both from the `.mcr` census. `CardCmd/<Transform>d__13`
    /// reads each original's `IndexOf` after the EARLIER originals were
    /// removed, then inserts the sorted records with `AddInternal(card, index)`.
    #[test]
    fn compact_fuel_lands_at_post_removal_indexes_like_native() {
        use CardId::{Dazed, DefendDefect, Fuel, Glasswork, Toxic};
        // f990b2953692f46b step 15: Dazed at 0, 2, 3, 5 record 0, 1, 1, 2.
        // Native Hand afterwards: Fuel x4, Defend, Glasswork.
        assert_eq!(
            compact_hand_after(&[Dazed, DefendDefect, Dazed, Dazed, Glasswork, Dazed]),
            vec![
                (100, Fuel),
                (102, Fuel),
                (103, Fuel),
                (101, Fuel),
                (11, DefendDefect),
                (14, Glasswork),
            ]
        );
        // f1b2172ac6278631: two leading Toxics both record index 0, so the
        // SECOND Fuel is inserted ahead of the first — the later native plays
        // of "the first Fuel" pick the Fuel minted second.
        assert_eq!(
            compact_hand_after(&[Toxic, Toxic, DefendDefect]),
            vec![(101, Fuel), (100, Fuel), (12, DefendDefect)]
        );
        // A trailing Status keeps its slot; nothing else moves.
        assert_eq!(
            compact_hand_after(&[DefendDefect, Glasswork, Dazed]),
            vec![(10, DefendDefect), (11, Glasswork), (100, Fuel)]
        );
    }

    /// At and one below the ten-card Hand cap the plural Transform never
    /// consults capacity: every original leaves before any Fuel is inserted,
    /// so the Hand ends exactly as large as it began and nothing reaches
    /// Discard.
    #[test]
    fn compact_post_removal_indexes_hold_at_and_below_hand_capacity() {
        use CardId::{Dazed, DefendDefect, Fuel, StrikeDefect};
        let nine = [
            StrikeDefect,
            Dazed,
            DefendDefect,
            Dazed,
            StrikeDefect,
            DefendDefect,
            Dazed,
            StrikeDefect,
            Dazed,
        ];
        // Dazed at 1, 3, 6, 8 record 1, 2, 4, 5.
        assert_eq!(
            compact_hand_after(&nine),
            vec![
                (10, StrikeDefect),
                (100, Fuel),
                (101, Fuel),
                (12, DefendDefect),
                (102, Fuel),
                (103, Fuel),
                (14, StrikeDefect),
                (15, DefendDefect),
                (17, StrikeDefect),
            ]
        );
        let ten = [Dazed; 10];
        let after = compact_hand_after(&ten);
        assert_eq!(after.len(), crate::engine::draw::MAX_CARDS_IN_HAND);
        // Every Dazed records index 0, so the Fuels land newest-first.
        assert_eq!(
            after.iter().map(|(uid, _)| *uid).collect::<Vec<_>>(),
            (100..110).rev().collect::<Vec<_>>()
        );
        assert!(after.iter().all(|(_, id)| *id == Fuel));
    }

    #[test]
    fn compact_refuses_a_hopper_master_status_before_public_play_prefix() {
        let (mut state, catalog, _source) = compact_parts(0);
        let source = state.piles.get_mut(PileId::Play).make_mut().pop().unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.exact_piles = true;
        let burn = state.piles.get(PileId::Hand).as_slice()[1];
        state.card_states.set_hopper(Some(HopperDeckState {
            master: vec![HopperDeckRow {
                card: burn,
                state: state.card_states.get(burn.uid),
            }],
            history: Vec::new(),
        }));
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
        state.monsters_mut()[0] = hopper;

        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let events_before = events.clone();
        assert_eq!(
            play_card(&mut state, &catalog, source.uid, None, None, &mut events,),
            Err(EngineRefusal::MalformedArgs(
                "persistent encounter-card transform"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn compact_generated_callbacks_are_serial_after_the_bulk_insert() {
        let (mut state, catalog, source) = compact_parts(0);
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 3);
        state.powers.set(PowerId::Arsenal, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation, PowerId::Arsenal,])
        );

        run_compact(
            &mut state,
            &catalog,
            source,
            &[CompiledArg::I(6), CompiledArg::I(0)],
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.block, 12, "two Pillar callbacks follow Compact Block");
        assert_eq!(state.powers.value(PowerId::Strength), 4);
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert_eq!(state.next_generated_hook_uid, 2);
    }

    /// Current Python `_batch225_after_card_changed_piles` (frozen, deleted #2827) derives the
    /// complete current-build listener set from the relic census: Bing Bong,
    /// Darkstone Periapt, and Lucky Fysh, all Deck-only. Compact inserts Fuel
    /// into Hand. Rust therefore may treat this native callback as inert
    /// only while admission refuses each of those relics. This complete-set
    /// assertion is intentionally local to the skipped fire point: when any
    /// listener is implemented, Compact must model the callback before this
    /// test can pass again.
    #[test]
    fn compact_discard_pile_change_callback_is_inert_under_admission() {
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
            "a current-build AfterCardChangedPiles listener became reachable; \
             Compact must publish and execute that callback"
        );
    }

    #[test]
    fn compact_terminal_gate_and_body_failures_are_atomic() {
        let (state, catalog, source) = compact_parts(0);

        let mut terminal = state.clone();
        terminal.history.over = true;
        let before = terminal.clone();
        run_compact(
            &mut terminal,
            &catalog,
            source,
            &[CompiledArg::I(6), CompiledArg::I(0)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(terminal, before);

        let mut lethal = state.clone();
        lethal.monsters_mut()[0].hp = 1;
        lethal.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            lethal
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let hand_before = lethal.piles.get(PileId::Hand).clone();
        run_compact(
            &mut lethal,
            &catalog,
            source,
            &[CompiledArg::I(6), CompiledArg::I(0)],
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.block, 6);
        assert_eq!(lethal.piles.get(PileId::Hand), &hand_before);
        assert_eq!(lethal.next_card_uid, 6);
        assert_eq!(lethal.history.owner_generated_cards_combat, 0);
        assert_eq!(lethal.next_generated_hook_uid, 0);

        for (owner_history, expected) in [
            (true, "owner_generated_cards_combat"),
            (false, "next_generated_hook_uid"),
        ] {
            let mut refusing = state.clone();
            if owner_history {
                refusing.history.owner_generated_cards_combat = i32::MAX;
            } else {
                refusing.next_generated_hook_uid = i32::MAX;
            }
            let before = refusing.clone();
            let mut events = vec![Event::CardResolved {
                uid: 99,
                pile: PileId::Draw,
            }];
            let events_before = events.clone();
            assert_eq!(
                run_compact(
                    &mut refusing,
                    &catalog,
                    source,
                    &[CompiledArg::I(6), CompiledArg::I(0)],
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow(expected))
            );
            assert_eq!(refusing, before);
            assert_eq!(events, events_before);
        }

        let mut malformed = state.clone();
        let before = malformed.clone();
        assert_eq!(
            run_compact(
                &mut malformed,
                &catalog,
                source,
                &[CompiledArg::I(7), CompiledArg::I(0)],
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs("compact_exact"))
        );
        assert_eq!(malformed, before);
    }

    #[test]
    fn compact_public_play_rolls_back_a_post_transform_refusal_and_burst_replays_block() {
        let (mut state, catalog, source) = compact_parts(0);
        let source_index = state
            .piles
            .get(PileId::Play)
            .as_slice()
            .iter()
            .position(|card| card.uid == source.uid)
            .unwrap();
        let source = state
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .remove(source_index);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.history.card_plays_finished_combat = i32::MAX;
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let events_before = events.clone();
        assert_eq!(
            play_card(&mut state, &catalog, source.uid, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        let (mut burst, catalog, source) = compact_parts(0);
        burst.piles.get_mut(PileId::Play).make_mut().clear();
        burst.piles.get_mut(PileId::Hand).make_mut().push(source);
        burst.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut burst);
        play_card(
            &mut burst,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(burst.block, 12);
        assert_eq!(burst.history.owner_generated_cards_combat, 2);
        assert_eq!(burst.next_card_uid, 8);
    }

    fn energy_surge_parts(upgrade: u8) -> (HotState, Catalog, HotCard) {
        let identity = identity(CardId::EnergySurge, upgrade);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        // GainEnergy checks IsEnding, which is trivially true for a
        // monsterless state; Energy Surge is played in live combat.
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            200,
        ));
        state.energy = 10;
        state.multiplayer_ally_key = 1;
        state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            energy: 4,
            ..MultiplayerAllyState::default()
        });
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        (state, catalog, source)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_energy_surge(
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
        energy_surge_exact(&mut StepCtx {
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
    fn energy_surge_both_rows_grant_every_living_player_the_exact_amount() {
        for (upgrade, amount) in [(0, 2), (1, 3)] {
            let row = crate::content_tables::card_row(CardId::EnergySurge, upgrade).unwrap();
            assert_eq!((row.cost, row.pool), (1, Some("Defect")));
            assert!(row.is_skill && row.exhausts && !row.targeted);
            assert_eq!(row.target_type, "AllAllies");
            assert_eq!(row.steps.len(), 1);
            assert_eq!(row.steps[0].kind, StepKind::EnergySurgeExact);

            let (mut state, catalog, source) = energy_surge_parts(upgrade);
            let mut events = Vec::new();
            run_energy_surge(
                &mut state,
                &catalog,
                source,
                None,
                None,
                0,
                &[CompiledArg::I(i64::from(amount))],
                &mut events,
            )
            .unwrap();

            assert_eq!(state.energy, 10 + amount as i16);
            assert_eq!(state.fanouts.multiplayer_ally().energy, 4 + amount);
            assert!(events.is_empty());
        }
        assert!(
            crate::content_tables::GENERATION_POOLS
                .iter()
                .all(|(_, pool)| !pool.contains(&CardId::EnergySurge)),
            "no current generated-card pool can create the MultiplayerOnly Skill"
        );

        let (mut direct_draw, catalog, source) = energy_surge_parts(0);
        direct_draw.piles.get_mut(PileId::Play).make_mut().clear();
        direct_draw
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(source);
        run_energy_surge(
            &mut direct_draw,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(direct_draw.energy, 12);
        assert_eq!(direct_draw.fanouts.multiplayer_ally().energy, 6);
        assert_eq!(direct_draw.piles.get(PileId::Draw).as_slice(), &[source]);
    }

    #[test]
    fn energy_surge_filters_dead_players_and_keeps_no_energy_gain_recipient_local() {
        let (mut blocked, catalog, source) = energy_surge_parts(0);
        blocked.fanouts.multiplayer_ally_mut().no_energy_gain = true;
        run_energy_surge(
            &mut blocked,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(blocked.energy, 12);
        assert_eq!(blocked.fanouts.multiplayer_ally().energy, 4);

        let (mut dead_remote, catalog, source) = energy_surge_parts(0);
        dead_remote.fanouts.multiplayer_ally_mut().alive = false;
        run_energy_surge(
            &mut dead_remote,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(dead_remote.energy, 12);
        assert_eq!(dead_remote.fanouts.multiplayer_ally().energy, 4);

        let (mut dead_owner, catalog, source) = energy_surge_parts(0);
        dead_owner.hp = 0;
        run_energy_surge(
            &mut dead_owner,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(2)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(dead_owner.energy, 10);
        assert_eq!(dead_owner.fanouts.multiplayer_ally().energy, 6);
    }

    #[test]
    fn energy_surge_terminal_and_cap_semantics_are_command_local() {
        let (mut terminal, catalog, source) = energy_surge_parts(1);
        terminal.history.over = true;
        let before = terminal.clone();
        run_energy_surge(
            &mut terminal,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(3)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(terminal, before);

        // #2708: PlayerCmd.GainEnergy 0x3ee8a0 returns at IsEnding (IL_0035),
        // not at the over latch. Every primary dead without a veto, before
        // history.over latches: neither living player gains.
        let (mut ending, catalog, source) = energy_surge_parts(1);
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let before = ending.clone();
        run_energy_surge(
            &mut ending,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(3)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(ending, before);

        let (mut capped_remote, catalog, source) = energy_surge_parts(1);
        capped_remote.fanouts.multiplayer_ally_mut().energy = 999_999_998;
        run_energy_surge(
            &mut capped_remote,
            &catalog,
            source,
            None,
            None,
            0,
            &[CompiledArg::I(3)],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(capped_remote.energy, 13);
        assert_eq!(capped_remote.fanouts.multiplayer_ally().energy, 999_999_999);
    }

    #[test]
    fn energy_surge_refuses_overflow_shape_and_source_drift_atomically() {
        let (mut overflow, catalog, source) = energy_surge_parts(0);
        overflow.energy = i16::MAX;
        let before = overflow.clone();
        let mut events = vec![Event::CardResolved {
            uid: 99,
            pile: PileId::Draw,
        }];
        let before_events = events.clone();
        assert_eq!(
            run_energy_surge(
                &mut overflow,
                &catalog,
                source,
                None,
                None,
                0,
                &[CompiledArg::I(2)],
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, before_events);

        for (target, selection, x_value, args) in [
            (Some(0), None, 0, vec![CompiledArg::I(2)]),
            (None, Some(1), 0, vec![CompiledArg::I(2)]),
            (None, None, 1, vec![CompiledArg::I(2)]),
            (None, None, 0, vec![CompiledArg::I(3)]),
        ] {
            let (mut malformed, catalog, source) = energy_surge_parts(0);
            let before = malformed.clone();
            assert_eq!(
                run_energy_surge(
                    &mut malformed,
                    &catalog,
                    source,
                    target,
                    selection,
                    x_value,
                    &args,
                    &mut Vec::new(),
                ),
                Err(EngineRefusal::MalformedArgs("energy_surge_exact"))
            );
            assert_eq!(malformed, before);
        }

        let (mut duplicate, catalog, source) = energy_surge_parts(0);
        duplicate
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        assert_eq!(
            run_energy_surge(
                &mut duplicate,
                &catalog,
                source,
                None,
                None,
                0,
                &[CompiledArg::I(2)],
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
    fn double_energy_snapshots_and_doubles_the_live_remainder() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.energy = 2;
        run(
            StepKind::GainCurrentEnergyExact,
            identity(CardId::DoubleEnergy, 0),
            &[],
            None,
            &mut state,
        )
        .unwrap();
        assert_eq!(state.energy, 4);

        state.energy = 0;
        run(
            StepKind::GainCurrentEnergyExact,
            identity(CardId::DoubleEnergy, 1),
            &[],
            None,
            &mut state,
        )
        .unwrap();
        assert_eq!(state.energy, 0);

        state.energy = 3;
        state.history.over = true;
        run(
            StepKind::GainCurrentEnergyExact,
            identity(CardId::DoubleEnergy, 1),
            &[],
            None,
            &mut state,
        )
        .unwrap();
        assert_eq!(state.energy, 3);
    }

    #[test]
    fn double_energy_pins_its_source_and_empty_argument_shape() {
        let mut state = HotState::at_defaults();
        assert_eq!(
            run(
                StepKind::GainCurrentEnergyExact,
                identity(CardId::DoubleEnergy, 0),
                &[CompiledArg::I(1)],
                None,
                &mut state,
            ),
            Err(EngineRefusal::MalformedArgs("gain_current_energy_exact"))
        );
        assert_eq!(
            run(
                StepKind::GainCurrentEnergyExact,
                identity(CardId::Zap, 0),
                &[],
                None,
                &mut state,
            ),
            Err(EngineRefusal::MalformedArgs("gain_current_energy_exact"))
        );
    }

    #[test]
    fn chaos_public_play_and_burst_publish_each_authenticated_iteration() {
        let identity = identity(CardId::Chaos, 1);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state.rng.set(
            RngStream::CombatOrbs,
            RngStreamState {
                words: [29, 30, 31, 32],
                counter: 0,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        state = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 7,
                target: None,
                selection: SelectionRef::NONE,
            },
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.orbs.next_random_orb_progress_uid(), 4);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 4);
        assert_eq!(state.powers.value(PowerId::Burst), 0);
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 7);
        assert!(state.piles.get(PileId::Play).is_empty());
    }

    #[test]
    fn chaos_public_allocator_overflow_rolls_back_the_complete_play() {
        let identity = identity(CardId::Chaos, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.orbs.set_lifecycle_counters(u32::MAX, 0);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
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
            Err(EngineRefusal::CounterOverflow(
                "next_random_orb_progress_uid"
            ))
        );
        assert_eq!(state, before, "energy, source, and history are atomic");
        assert_eq!(events, before_events, "the public event prefix is atomic");
    }

    #[test]
    fn chaos_public_rng_counter_overflow_rolls_back_the_complete_play() {
        let identity = identity(CardId::Chaos, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state.rng.set(
            RngStream::CombatOrbs,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: u64::MAX,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 8,
            atom,
            flags: 0,
        });
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            play_card(&mut state, &catalog, 8, None, None, &mut events),
            Err(EngineRefusal::CounterOverflow("random orb bound"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn catastrophe_auto_plays_chaos_from_draw_without_weakening_source_authentication() {
        let catastrophe = identity(CardId::Catastrophe, 0);
        let chaos = identity(CardId::Chaos, 0);
        let mut builder = CatalogBuilder::new();
        let catastrophe_atom = builder.intern(catastrophe).unwrap();
        let chaos_atom = builder.intern(chaos).unwrap();
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
            atom: chaos_atom,
            flags: 0,
        });
        let mut events = Vec::new();

        play_card(&mut state, &catalog, 40, None, None, &mut events).unwrap();

        assert_eq!(state.orbs.next_random_orb_progress_uid(), 1);
        assert_eq!(state.rng.get(RngStream::CombatOrbs).counter, 1);
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(state.piles.get(PileId::Discard).as_slice()[0].uid, 41);
    }

    #[test]
    fn chaos_forged_carrier_program_refuses_atomically() {
        let chaos = identity(CardId::Chaos, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(chaos).unwrap();
        let catalog = builder.build();
        let mut spec = *catalog.spec(atom).unwrap();
        spec.row = crate::content_tables::card_row(CardId::WhiteNoise, 0).unwrap();
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
            random_orb_loop_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("random_orb_loop_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn scrape_filters_native_nonzero_or_x_returned_costs_at_both_levels() {
        let zero = identity(CardId::Zap, 1);
        let live_free = identity(CardId::DefendDefect, 0);
        let x = identity(CardId::Tempest, 0);
        let positive = identity(CardId::DefendDefect, 0);
        let negative = identity(CardId::Wound, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Scrape, 0)).unwrap();
        builder.intern(identity(CardId::Scrape, 1)).unwrap();
        let zero_atom = builder.intern(zero).unwrap();
        let live_free_atom = builder.intern(live_free).unwrap();
        let x_atom = builder.intern(x).unwrap();
        let positive_atom = builder.intern(positive).unwrap();
        let negative_atom = builder.intern(negative).unwrap();
        let catalog = builder.build();

        for level in [0, 1] {
            let source = identity(CardId::Scrape, level);
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 40));
            state.next_card_uid = 100;
            // UID 2 costs zero only after its live modifier. UID 5 begins at
            // zero but resolves positive after its live modifier. Tempest
            // resolves X as zero at zero energy and is still selected.
            for (uid, amount) in [(2, 0), (5, 2)] {
                state.card_states.append_local_cost_modifier(
                    uid,
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Set,
                        amount,
                        expiration: LocalCostExpiration::ThisTurnOrPlayed,
                        reduce_only: false,
                    },
                );
            }
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 99,
                atom: positive_atom,
                flags: 0,
            });
            let returned = if level == 0 {
                vec![
                    HotCard {
                        uid: 1,
                        atom: zero_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 3,
                        atom: x_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 6,
                        atom: negative_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 4,
                        atom: positive_atom,
                        flags: 0,
                    },
                ]
            } else {
                vec![
                    HotCard {
                        uid: 1,
                        atom: zero_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 2,
                        atom: live_free_atom,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    HotCard {
                        uid: 3,
                        atom: x_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 4,
                        atom: positive_atom,
                        flags: 0,
                    },
                    HotCard {
                        uid: 5,
                        atom: zero_atom,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                ]
            };
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(returned);

            run_with_catalog(
                StepKind::ScrapeExact,
                source,
                &[
                    CompiledArg::I(if level == 0 { 7 } else { 10 }),
                    CompiledArg::I(4_i64 + i64::from(level)),
                ],
                Some(0),
                &mut state,
                &catalog,
            )
            .unwrap();

            assert_eq!(state.monsters[0].hp, 40 - if level == 0 { 7 } else { 10 });
            assert!(state.exact_piles);
            assert_eq!(
                state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                if level == 0 {
                    vec![99, 1]
                } else {
                    vec![99, 1, 2]
                },
                "only returned cards are eligible for Scrape's filter"
            );
            let expected_discard = if level == 0 {
                vec![3, 6, 4]
            } else {
                vec![3, 4, 5]
            };
            assert_eq!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                expected_discard
            );
            assert_eq!(state.history.discarded_cards_this_turn, 3);
        }
    }

    #[test]
    fn lethal_scrape_suppresses_normalization_draw_and_cost_filter() {
        let source = identity(CardId::Scrape, 0);
        let draw = identity(CardId::Zap, 1);
        let mut builder = CatalogBuilder::new();
        builder.intern(source).unwrap();
        let draw_atom = builder.intern(draw).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 7));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 0,
            atom: draw_atom,
            flags: 0,
        });
        let before = state.rng.get(RngStream::Rng);

        run_with_catalog(
            StepKind::ScrapeExact,
            source,
            &[CompiledArg::I(7), CompiledArg::I(4)],
            Some(0),
            &mut state,
            &catalog,
        )
        .unwrap();

        assert!(state.history.over);
        assert!(!state.exact_piles);
        assert_eq!(state.piles.get(PileId::Draw).as_slice().len(), 1);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.rng.get(RngStream::Rng), before);
    }

    #[test]
    fn scrape_rejects_bad_shape_and_missing_target_before_mutation() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let before = state.clone();
        assert_eq!(
            run(
                StepKind::ScrapeExact,
                identity(CardId::Scrape, 0),
                &[CompiledArg::I(7), CompiledArg::I(5)],
                Some(0),
                &mut state,
            ),
            Err(EngineRefusal::MalformedArgs("scrape_exact"))
        );
        assert_eq!(state, before);
        assert_eq!(
            run(
                StepKind::ScrapeExact,
                identity(CardId::Scrape, 0),
                &[CompiledArg::I(7), CompiledArg::I(4)],
                None,
                &mut state,
            ),
            Err(EngineRefusal::TargetMismatch { required: true })
        );
        assert_eq!(state, before);
    }

    fn white_noise_fixture(
        owner: RewardPool,
        upgrade: u8,
        seed: u64,
    ) -> (HotState, Catalog, CardIdentity, RngStreamState) {
        let source_identity = identity(CardId::WhiteNoise, upgrade);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(source_identity).unwrap();
        let pool = owner_power_pool(owner);
        for id in pool {
            builder.intern(identity(*id, 0)).unwrap();
        }
        let catalog = builder.build();
        let seeded = Xoshiro256StarStar::from_seed(seed);
        let entering = RngStreamState {
            words: seeded.words,
            counter: seeded.counter,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.reward_card_pool = Some(owner);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(RngStream::Generation, entering);
        state.next_card_uid = 18;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 17,
            atom,
            flags: 0,
        });
        (state, catalog, source_identity, entering)
    }

    fn run_white_noise_body(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 17,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events,
        };
        white_noise_exact(&mut ctx)
    }

    #[test]
    fn white_noise_uses_every_exact_owner_pool_and_generates_one_free_l0_power() {
        for (owner, expected_len) in [
            (RewardPool::Ironclad, 18usize),
            (RewardPool::Silent, 16),
            (RewardPool::Regent, 16),
            (RewardPool::Necrobinder, 17),
            (RewardPool::Defect, 19),
        ] {
            for upgrade in 0..=1 {
                let (mut state, catalog, source, entering) =
                    white_noise_fixture(owner, upgrade, 47);
                state.powers.set(PowerId::Arsenal, SlotWire::Int, 2);
                assert!(
                    state
                        .fanouts
                        .set_local_generated_power_order(&[PowerId::Arsenal])
                );
                let mut oracle = Xoshiro256StarStar {
                    words: entering.words,
                    counter: entering.counter,
                };
                let mut pool = owner_power_pool(owner).to_vec();
                oracle.shuffle(&mut pool).unwrap();
                let mut events = Vec::new();

                run_white_noise_body(&mut state, &catalog, source, &mut events).unwrap();

                assert_eq!(pool.len(), expected_len);
                assert_eq!(
                    state.rng.get(RngStream::Generation).counter,
                    entering.counter + u64::try_from(expected_len - 1).unwrap()
                );
                assert_eq!(state.history.owner_generated_cards_combat, 1);
                assert_eq!(state.next_generated_hook_uid, 1);
                assert_eq!(state.next_card_uid, 19);
                assert_eq!(state.powers.value(PowerId::Strength), 2);
                let generated = state.piles.get(PileId::Hand).as_slice()[0];
                assert_eq!(generated.uid, 18);
                assert_ne!(generated.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
                assert_eq!(
                    catalog.spec(generated.atom).unwrap().identity,
                    identity(pool[0], 0),
                    "White Noise+ changes only the source Energy cost"
                );
                let instance = state.card_states.get(generated.uid);
                assert_eq!(instance.local_cost_modifiers.resolve(99), 0);
                assert_eq!(
                    instance.local_cost_modifiers.as_slice()[0].expiration,
                    LocalCostExpiration::ThisTurnOrPlayed
                );
                assert_eq!(instance.free_star_cost_this_turn_or_played_rows, 1);
                assert_eq!(
                    events,
                    vec![
                        Event::CardResolved {
                            uid: 18,
                            pile: PileId::Hand,
                        },
                        Event::PowerChanged {
                            subject: crate::engine::Subject::Player,
                            power: PowerId::Strength,
                            amount: 2,
                        },
                    ]
                );
            }
        }
    }

    #[test]
    fn white_noise_full_hand_and_terminal_paths_preserve_shared_generated_order() {
        let (mut full, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        let filler = catalog.atom(&identity(CardId::WhiteNoise, 0)).unwrap();
        full.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((20..30).map(|uid| HotCard {
                uid,
                atom: filler,
                flags: 0,
            }));
        let mut events = Vec::new();
        run_white_noise_body(&mut full, &catalog, source, &mut events).unwrap();
        assert_eq!(full.piles.get(PileId::Hand).len(), 10);
        assert_eq!(full.piles.get(PileId::Discard).as_slice()[0].uid, 18);
        assert_eq!(
            events,
            vec![Event::CardResolved {
                uid: 18,
                pile: PileId::Discard,
            }]
        );

        let (mut terminal, catalog, source, entering) =
            white_noise_fixture(RewardPool::Defect, 0, 47);
        terminal.history.over = true;
        terminal.history.owner_generated_cards_combat = 3;
        terminal.next_generated_hook_uid = 5;
        let before_piles = terminal.piles.clone();
        let mut events = Vec::new();
        run_white_noise_body(&mut terminal, &catalog, source, &mut events).unwrap();
        assert_eq!(
            terminal.rng.get(RngStream::Generation).counter,
            entering.counter + 18
        );
        assert_eq!(terminal.history.owner_generated_cards_combat, 4);
        assert_eq!(terminal.next_generated_hook_uid, 6);
        assert_eq!(terminal.next_card_uid, 18);
        assert_eq!(terminal.piles, before_piles);
        assert!(!terminal.exact_piles);
        assert!(events.is_empty());
    }

    #[test]
    fn white_noise_accepts_non_null_sources_and_refuses_invalid_provenance_atomically() {
        let (mut missing, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        missing.piles.get_mut(PileId::Play).make_mut().clear();
        let before = missing.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_white_noise_body(&mut missing, &catalog, source, &mut events),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: 17,
                matches: 0,
            })
        );
        assert_eq!(missing, before);
        assert!(events.is_empty());

        let (mut duplicate, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        let duplicate_card = duplicate.piles.get(PileId::Play).as_slice()[0];
        duplicate
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(duplicate_card);
        let before = duplicate.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_white_noise_body(&mut duplicate, &catalog, source, &mut events),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: 17,
                matches: 2,
            })
        );
        assert_eq!(duplicate, before);
        assert!(events.is_empty());

        for source_pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
            let (mut direct, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
            let source_card = direct.piles.get_mut(PileId::Play).make_mut().remove(0);
            direct
                .piles
                .get_mut(source_pile)
                .make_mut()
                .push(source_card);
            let mut events = Vec::new();
            run_white_noise_body(&mut direct, &catalog, source, &mut events).unwrap();
            assert!(
                direct
                    .piles
                    .get(source_pile)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == source_card.uid)
            );
            assert_eq!(direct.history.owner_generated_cards_combat, 1);
        }

        let (mut wrong_atom, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        wrong_atom.piles.get_mut(PileId::Play).make_mut()[0].atom = catalog
            .atom(&identity(owner_power_pool(RewardPool::Defect)[0], 0))
            .unwrap();
        let before = wrong_atom.clone();
        let mut events = Vec::new();
        assert_eq!(
            run_white_noise_body(&mut wrong_atom, &catalog, source, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "white noise generation provenance"
            ))
        );
        assert_eq!(wrong_atom, before);
        assert!(events.is_empty());

        let (mut state, catalog, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        let spec = *catalog.spec(catalog.atom(&source).unwrap()).unwrap();
        let before = state.clone();
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
            white_noise_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("white_noise_exact"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.multiplayer_ally_key = 1;
        let party = state.clone();
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
            white_noise_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs(
                "white noise generation provenance"
            ))
        );
        assert_eq!(state, party);
        assert!(events.is_empty());

        let (mut state, full, source, _) = white_noise_fixture(RewardPool::Defect, 0, 47);
        let live = state.piles.get(PileId::Play).as_slice()[0];
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source).unwrap();
        let narrow = builder.build();
        state.piles.get_mut(PileId::Play).make_mut()[0] = HotCard {
            atom: source_atom,
            ..live
        };
        let before = state.clone();
        let mut events = Vec::new();
        let result = run_white_noise_body(&mut state, &narrow, source, &mut events);
        assert!(matches!(result, Err(EngineRefusal::UnknownMintIdentity(_))));
        assert_eq!(state, before);
        assert!(events.is_empty());
        assert!(full.specs().count() > narrow.specs().count());
    }

    #[test]
    fn white_noise_manual_direct_autoplay_and_burst_share_the_exact_body() {
        for (mode, burst, expected_cards, expected_draws) in [
            (0u8, 0, 1usize, 18u64),
            (1, 0, 1, 18),
            (2, 0, 1, 18),
            (0, 1, 2, 36),
        ] {
            let (mut state, catalog, identity, entering) =
                white_noise_fixture(RewardPool::Defect, 0, 47);
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            let source = state.piles.get_mut(PileId::Play).make_mut().remove(0);
            let initial_pile = if mode == 0 {
                PileId::Hand
            } else {
                PileId::Draw
            };
            state.piles.get_mut(initial_pile).make_mut().push(source);
            state.energy = 3;
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
                1
            );
            assert!(state.piles.get(PileId::Play).is_empty());
            assert_eq!(catalog.spec(source.atom).unwrap().identity, identity);
        }
    }

    fn hibernate_parts(upgrade: u8, pile: PileId) -> (HotState, Catalog, HotCard) {
        let identity = identity(CardId::Hibernate, upgrade);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 278,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
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
            alive: true,
            ..MultiplayerAllyState::default()
        });
        state.orbs.set_base_slots(1);
        state.orbs.set_slots(1);
        state
            .orbs
            .set_orbs(vec![HotOrb::from_parts(OrbKind::Frost, None).unwrap()]);
        state.piles.get_mut(pile).make_mut().push(source);
        (state, catalog, source)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_hibernate(
        state: &mut HotState,
        catalog: &Catalog,
        source: HotCard,
        spec: CardSpec,
        args: &[CompiledArg],
        target: Option<usize>,
        selection: Option<u32>,
        x_value: i64,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        hibernate_power_exact(&mut StepCtx {
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
    fn hibernate_both_levels_apply_one_then_channel_the_exact_frost_suffix() {
        for (upgrade, frost_count) in [(0, 2), (1, 3)] {
            let row = crate::content_tables::card_row(CardId::Hibernate, upgrade).unwrap();
            assert_eq!((row.cost, row.pool), (2, Some("Defect")));
            assert!(row.is_skill && !row.exhausts && !row.targeted);
            assert_eq!(row.target_type, "Self");
            assert_eq!(row.steps.len(), 1 + frost_count);
            assert_eq!(row.steps[0].kind, StepKind::HibernatePowerExact);
            assert!(row.steps[1..].iter().all(|step| {
                step.kind == StepKind::Channel
                    && step.args
                        == [
                            crate::content_tables::Arg::S("FROST"),
                            crate::content_tables::Arg::I(1),
                        ]
            }));

            let (mut state, catalog, source) = hibernate_parts(upgrade, PileId::Hand);
            // Channel checks IsEnding, which is trivially true for a
            // monsterless state; Hibernate is played in live combat.
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            assert!(catalog.spec(source.atom).unwrap().solo_unplayable);
            let mut events = Vec::new();
            play_card(&mut state, &catalog, source.uid, None, None, &mut events).unwrap();

            assert_eq!(state.energy, 1);
            assert_eq!(state.powers.value(PowerId::Hibernate), 1);
            assert_eq!(state.block, 5 * frost_count as i32);
            assert_eq!(state.orbs.as_slice().len(), 1);
            assert_eq!(state.orbs.as_slice()[0].kind(), OrbKind::Frost);
            assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
            assert!(events.contains(&Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::Hibernate,
                amount: 1,
            }));
        }
    }

    #[test]
    fn hibernate_late_channel_refusal_rolls_back_all_play_and_replay_routes() {
        for upgrade in 0..=1 {
            let frost_count = if upgrade == 0 { 2 } else { 3 };
            for (power, autoplay) in [
                (None, false),
                (None, true),
                (Some(PowerId::Burst), false),
                (Some(PowerId::EchoForm), false),
                (Some(PowerId::Burst), true),
            ] {
                let pile = if autoplay { PileId::Draw } else { PileId::Hand };
                let (mut state, catalog, source) = hibernate_parts(upgrade, pile);
                // Channel checks IsEnding, which is trivially true for a
                // monsterless state; Hibernate is played in live combat.
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                let has_later_body = power.is_some();
                let successful_channels = if has_later_body {
                    frost_count
                } else {
                    frost_count - 1
                };
                state.block = i32::MAX - 5 * successful_channels;
                if let Some(power) = power {
                    state.powers.set(power, SlotWire::Int, 1);
                }
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
                let before = state.clone();
                let mut events = vec![Event::TurnBegan { turn: 278 }];
                let before_events = events.clone();

                let result = if autoplay {
                    autoplay_collected_cards(&mut state, &catalog, &[source], &mut events)
                } else {
                    play_card(&mut state, &catalog, source.uid, None, None, &mut events)
                };

                assert_eq!(
                    result,
                    Err(EngineRefusal::CounterOverflow("player block")),
                    "upgrade={upgrade} power={power:?} autoplay={autoplay}"
                );
                assert_eq!(
                    state, before,
                    "power, spend, piles, history, queue, and replay state roll back"
                );
                assert_eq!(events, before_events, "no event prefix escapes");
            }
        }
    }

    #[test]
    fn hibernate_foreign_base_replay_count_refuses_before_any_prefix() {
        for upgrade in 0..=1 {
            let (mut state, catalog, source) = hibernate_parts(upgrade, PileId::Hand);
            state.piles.get_mut(PileId::Hand).make_mut()[0].flags |=
                crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE;
            let mut instance = state.card_states.get(source.uid);
            instance.set_base_replay_count(Some(1)).unwrap();
            state.card_states.set(source.uid, instance);
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 278 }];
            let before_events = events.clone();

            assert_eq!(
                play_card(&mut state, &catalog, source.uid, None, None, &mut events,),
                Err(EngineRefusal::MalformedArgs("hibernate physical source"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn hibernate_application_refuses_malformed_inputs_and_is_ending_gated() {
        let (mut ending, catalog, source) = hibernate_parts(0, PileId::Play);
        ending.history.over = true;
        let spec = *catalog.spec(source.atom).unwrap();
        let before = ending.clone();
        let mut events = Vec::new();
        call_hibernate(
            &mut ending,
            &catalog,
            source,
            spec,
            &[CompiledArg::I(1)],
            None,
            None,
            0,
            &mut events,
        )
        .unwrap();
        assert_eq!(ending, before);
        assert!(events.is_empty());

        for (target, selection, x_value, args) in [
            (Some(0), None, 0, vec![CompiledArg::I(1)]),
            (None, Some(0), 0, vec![CompiledArg::I(1)]),
            (None, None, 1, vec![CompiledArg::I(1)]),
            (None, None, 0, vec![CompiledArg::I(2)]),
        ] {
            let (mut state, catalog, source) = hibernate_parts(0, PileId::Play);
            let spec = *catalog.spec(source.atom).unwrap();
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                call_hibernate(
                    &mut state,
                    &catalog,
                    source,
                    spec,
                    &args,
                    target,
                    selection,
                    x_value,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("hibernate_power_exact"))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        for wire in [SlotWire::Bool, SlotWire::Int] {
            let (mut state, catalog, source) = hibernate_parts(0, PileId::Play);
            state.powers.set(
                PowerId::Hibernate,
                wire,
                if wire == SlotWire::Bool { 1 } else { i32::MAX },
            );
            let spec = *catalog.spec(source.atom).unwrap();
            let before = state.clone();
            let mut events = Vec::new();
            assert!(
                call_hibernate(
                    &mut state,
                    &catalog,
                    source,
                    spec,
                    &[CompiledArg::I(1)],
                    None,
                    None,
                    0,
                    &mut events,
                )
                .is_err()
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let (mut duplicate, catalog, source) = hibernate_parts(0, PileId::Play);
        duplicate
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let spec = *catalog.spec(source.atom).unwrap();
        let before = duplicate.clone();
        let mut events = Vec::new();
        assert_eq!(
            call_hibernate(
                &mut duplicate,
                &catalog,
                source,
                spec,
                &[CompiledArg::I(1)],
                None,
                None,
                0,
                &mut events,
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(duplicate, before);
        assert!(events.is_empty());

        let (mut legacy, catalog, source) = hibernate_parts(0, PileId::Play);
        legacy.piles.get_mut(PileId::Play).make_mut()[0].flags |= CARD_FLAG_LEGACY;
        let spec = *catalog.spec(source.atom).unwrap();
        let before = legacy.clone();
        let mut events = Vec::new();
        assert_eq!(
            call_hibernate(
                &mut legacy,
                &catalog,
                source,
                spec,
                &[CompiledArg::I(1)],
                None,
                None,
                0,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("hibernate physical source"))
        );
        assert_eq!(legacy, before);
        assert!(events.is_empty());

        let mut builder = CatalogBuilder::new();
        let hibernate_atom = builder.intern(identity(CardId::Hibernate, 0)).unwrap();
        let compact_atom = builder.intern(identity(CardId::Compact, 0)).unwrap();
        let mixed_catalog = builder.build();
        let source = HotCard {
            uid: 278,
            atom: compact_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut wrong_source = HotState::at_defaults();
        wrong_source.hp = 50;
        wrong_source
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(source);
        let hibernate_spec = *mixed_catalog.spec(hibernate_atom).unwrap();
        let before = wrong_source.clone();
        let mut events = Vec::new();
        assert_eq!(
            call_hibernate(
                &mut wrong_source,
                &mixed_catalog,
                source,
                hibernate_spec,
                &[CompiledArg::I(1)],
                None,
                None,
                0,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("hibernate physical source"))
        );
        assert_eq!(wrong_source, before);
        assert!(events.is_empty());

        let compact_spec = *mixed_catalog.spec(compact_atom).unwrap();
        assert_eq!(
            call_hibernate(
                &mut wrong_source,
                &mixed_catalog,
                source,
                compact_spec,
                &[CompiledArg::I(1)],
                None,
                None,
                0,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("hibernate program"))
        );
    }

    #[test]
    fn hibernate_turn_start_decrement_uses_the_native_ending_gate() {
        let mut ordinary = HotState::at_defaults();
        ordinary.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        ordinary.hp = 50;
        ordinary.powers.set(PowerId::Hibernate, SlotWire::Int, 1);
        let captured = preflight_hibernate_turn_start(&ordinary).unwrap();
        assert!(captured);
        let mut events = Vec::new();
        tick_hibernate_turn_start(&mut ordinary, captured, &mut events).unwrap();
        assert_eq!(ordinary.powers.value(PowerId::Hibernate), 0);
        assert_eq!(ordinary.powers.get(PowerId::Hibernate), None);
        assert_eq!(
            events,
            [Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::Hibernate,
                amount: 0,
            }]
        );

        let mut already_terminal = HotState::at_defaults();
        already_terminal.history.over = true;
        already_terminal
            .powers
            .set(PowerId::Hibernate, SlotWire::Int, 2);
        let captured = preflight_hibernate_turn_start(&already_terminal).unwrap();
        assert!(!captured);
        let before = already_terminal.clone();
        let mut events = Vec::new();
        tick_hibernate_turn_start(&mut already_terminal, captured, &mut events).unwrap();
        assert_eq!(already_terminal, before);
        assert!(events.is_empty());

        let mut ended_by_prior_listener = HotState::at_defaults();
        ended_by_prior_listener
            .monsters_mut()
            .push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
        ended_by_prior_listener.hp = 50;
        ended_by_prior_listener
            .powers
            .set(PowerId::Hibernate, SlotWire::Int, 2);
        let captured = preflight_hibernate_turn_start(&ended_by_prior_listener).unwrap();
        assert!(captured);
        ended_by_prior_listener.history.over = true;
        let before = ended_by_prior_listener.clone();
        let mut events = Vec::new();
        tick_hibernate_turn_start(&mut ended_by_prior_listener, captured, &mut events).unwrap();
        assert_eq!(ended_by_prior_listener, before);
        assert!(events.is_empty());

        let mut suspended = HotState::at_defaults();
        suspended.hp = 50;
        suspended.powers.set(PowerId::Hibernate, SlotWire::Int, 2);
        let record = crate::hot::CardPlayRecord::pending_for_test(278);
        let pending = suspended.frames.push_pending_for_test(&record);
        suspended.pending = Some(std::sync::Arc::new(pending));
        let before = suspended.clone();
        assert_eq!(
            preflight_hibernate_turn_start(&suspended),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(
            suspended, before,
            "a suspended hand-draw cannot be resumed approximately"
        );
    }

    #[test]
    fn hibernate_authenticates_every_type_one_listener_permutation() {
        let listeners = [
            PowerId::Vicious,
            PowerId::Shroud,
            PowerId::SleightOfFlesh,
            PowerId::SwordSage,
        ];
        for first in 0..4 {
            for second in 0..4 {
                for third in 0..4 {
                    for fourth in 0..4 {
                        let order = [first, second, third, fourth];
                        if order
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            != 4
                        {
                            continue;
                        }
                        let mut state = HotState::at_defaults();
                        state.hp = 50;
                        state.powers.set(PowerId::Hibernate, SlotWire::Int, 1);
                        for power in listeners {
                            state.powers.set(power, SlotWire::Int, 1);
                        }
                        let permutation = order.map(|index| listeners[index]);
                        assert!(
                            state
                                .fanouts
                                .set_after_power_amount_changed_order(&permutation)
                        );
                        assert_eq!(preflight_hibernate_turn_start(&state), Ok(true));
                    }
                }
            }
        }

        for (live, order) in [
            (None, vec![PowerId::Shroud]),
            (Some(PowerId::Shroud), vec![]),
            (
                Some(PowerId::Shroud),
                vec![PowerId::Shroud, PowerId::Shroud],
            ),
            (Some(PowerId::Shroud), vec![PowerId::Vicious]),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::Hibernate, SlotWire::Int, 1);
            if let Some(power) = live {
                state.powers.set(power, SlotWire::Int, 1);
            }
            assert!(state.fanouts.set_after_power_amount_changed_order(&order));
            assert_eq!(
                preflight_hibernate_turn_start(&state),
                Err(EngineRefusal::PowerOrderNotModeled(
                    "Hibernate power-amount event order"
                ))
            );
        }
    }

    #[test]
    fn family_manifest_and_all_sixteen_generated_carriers_are_exact() {
        const FAMILY_KINDS: [StepKind; 8] = [
            StepKind::CompactExact,
            StepKind::EnergySurgeExact,
            StepKind::GainCurrentEnergyExact,
            StepKind::HibernatePowerExact,
            StepKind::IgnitionExact,
            StepKind::RandomOrbLoopExact,
            StepKind::ScrapeExact,
            StepKind::WhiteNoiseExact,
        ];
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::CompactExact,
                StepKind::EnergySurgeExact,
                StepKind::GainCurrentEnergyExact,
                StepKind::HibernatePowerExact,
                StepKind::RandomOrbLoopExact,
                StepKind::ScrapeExact,
                StepKind::WhiteNoiseExact,
            ]
        );
        let carriers = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| FAMILY_KINDS.contains(&step.kind))
            })
            .collect::<Vec<_>>();
        assert_eq!(carriers.len(), 16, "eight exact cards at both levels");
        assert_eq!(
            carriers
                .iter()
                .map(|row| (row.id, row.upgrade, row.steps[0].kind))
                .collect::<Vec<_>>(),
            vec![
                (CardId::Chaos, 0, StepKind::RandomOrbLoopExact),
                (CardId::Chaos, 1, StepKind::RandomOrbLoopExact),
                (CardId::Compact, 0, StepKind::CompactExact),
                (CardId::Compact, 1, StepKind::CompactExact),
                (CardId::DoubleEnergy, 0, StepKind::GainCurrentEnergyExact),
                (CardId::DoubleEnergy, 1, StepKind::GainCurrentEnergyExact),
                (CardId::EnergySurge, 0, StepKind::EnergySurgeExact),
                (CardId::EnergySurge, 1, StepKind::EnergySurgeExact),
                (CardId::Hibernate, 0, StepKind::HibernatePowerExact),
                (CardId::Hibernate, 1, StepKind::HibernatePowerExact),
                (CardId::Ignition, 0, StepKind::IgnitionExact),
                (CardId::Ignition, 1, StepKind::IgnitionExact),
                (CardId::Scrape, 0, StepKind::ScrapeExact),
                (CardId::Scrape, 1, StepKind::ScrapeExact),
                (CardId::WhiteNoise, 0, StepKind::WhiteNoiseExact),
                (CardId::WhiteNoise, 1, StepKind::WhiteNoiseExact),
            ]
        );
        let admitted = carriers
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .all(|step| crate::engine::admission::step_is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![
                (CardId::Chaos, 0),
                (CardId::Chaos, 1),
                (CardId::Compact, 0),
                (CardId::Compact, 1),
                (CardId::DoubleEnergy, 0),
                (CardId::DoubleEnergy, 1),
                (CardId::EnergySurge, 0),
                (CardId::EnergySurge, 1),
                (CardId::Hibernate, 0),
                (CardId::Hibernate, 1),
                (CardId::Scrape, 0),
                (CardId::Scrape, 1),
                (CardId::WhiteNoise, 0),
                (CardId::WhiteNoise, 1),
            ]
        );
        let manifest = crate::engine::capability_manifest();
        for kind in FAMILY_KINDS {
            assert_eq!(
                manifest.steps.contains(&kind),
                IMPLEMENTED.contains(&kind),
                "family manifest drift for {kind:?}"
            );
        }
    }

    /// #3515: each body gates on its native command, not `history.over`,
    /// and is a complete no-op while the combat is ending before the over
    /// latch; the Adaptable-vetoed control writes:
    /// - Compact's plural `CardCmd.Transform` (`<Transform>d__13` 0x3e0ae0
    ///   IL_0032, `IsEnding`);
    /// - Double Energy's `PlayerCmd.GainEnergy` (`<GainEnergy>d__3` 0x3ee8a0
    ///   IL_0035, `IsEnding`);
    /// - Hibernate's `Apply<HibernatePower>` (`<Apply>d__1`1` 0x3ef988
    ///   IL_0025) and its turn-start `PowerCmd.Decrement` (`<ModifyAmount>d__6`
    ///   0x3f032c IL_003a), both `IsEnding`;
    /// - Scrape's Draw and Discard (`<DrawInternal>d__21` 0x3e3a70 IL_002e,
    ///   `<DiscardAndDraw>d__4` 0x3e0274 IL_002e, both `IsOverOrEnding`).
    #[test]
    fn defect_uncommon_bodies_skip_while_combat_is_ending_before_the_over_latch() {
        for upgrade in [0, 1] {
            let (template, catalog, source) = compact_parts(upgrade);
            let args = [
                CompiledArg::I(6 + i64::from(upgrade)),
                CompiledArg::I(i64::from(upgrade)),
            ];
            crate::engine::damage::assert_ending_window_gate(&template, "Compact", |s, _| {
                run_compact(s, &catalog, source, &args, &mut Vec::new())
            });
            // The live control's Block alone would change the state, so pin
            // the gated write: the Statuses became Fuel.
            let mut control = template.clone();
            control.monsters_mut().clear();
            crate::engine::damage::push_ending_window_roster(&mut control, true);
            run_compact(&mut control, &catalog, source, &args, &mut Vec::new()).unwrap();
            assert_ne!(
                control.piles.get(PileId::Hand).as_slice(),
                template.piles.get(PileId::Hand).as_slice()
            );
        }

        let mut template = HotState::at_defaults();
        template.energy = 2;
        crate::engine::damage::assert_ending_window_gate(&template, "Double Energy", |s, _| {
            run(
                StepKind::GainCurrentEnergyExact,
                identity(CardId::DoubleEnergy, 0),
                &[],
                None,
                s,
            )
        });

        let (template, catalog, source) = hibernate_parts(0, PileId::Play);
        let spec = *catalog.spec(source.atom).unwrap();
        crate::engine::damage::assert_ending_window_gate(&template, "Hibernate", |s, _| {
            call_hibernate(
                s,
                &catalog,
                source,
                spec,
                &[CompiledArg::I(1)],
                None,
                None,
                0,
                &mut Vec::new(),
            )
        });
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.powers.set(PowerId::Hibernate, SlotWire::Int, 2);
        crate::engine::damage::assert_ending_window_gate(&template, "Hibernate tick", |s, _| {
            tick_hibernate_turn_start(s, true, &mut Vec::new())
        });

        let scrape = identity(CardId::Scrape, 0);
        let strike = identity(CardId::StrikeDefect, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(scrape).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 0,
                atom: strike_atom,
                flags: 0,
            });
        crate::engine::damage::assert_ending_window_gate(&template, "Scrape", |s, t| {
            run_with_catalog(
                StepKind::ScrapeExact,
                scrape,
                &[CompiledArg::I(7), CompiledArg::I(4)],
                Some(t),
                s,
                &catalog,
            )
        });
        // The discard after a draw that left the combat ending: the drawn
        // Strike stays in Hand.
        let drawn = HotCard {
            uid: 0,
            atom: strike_atom,
            flags: 0,
        };
        let mut template = HotState::at_defaults();
        template.hp = 50;
        template.piles.get_mut(PileId::Hand).make_mut().push(drawn);
        crate::engine::damage::assert_ending_window_gate(&template, "Scrape discard", |s, _| {
            apply_scrape_after_draw(s, &catalog, &[drawn], &mut Vec::new())
        });
    }
}
