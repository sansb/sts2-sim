//! Card-step bodies for the `content/cards/regent_forge.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Current status: all 6 step kinds ported
//!
//! This family is downstream of Regent's shared Forge command, the mutable
//! Sovereign Blade physical-card payload, and four player powers with native
//! readers or hooks. Unit E (#1363) supplies the exact physical payload reader
//! for an already-materialized Sovereign Blade. This unit adds the exact Forge
//! transaction for every canonical Forge mode, including Hammer Time and
//! Sword Sage: Beat Into Shape, Big Bang, Bulwark, Conqueror, Refine Blade,
//! Seeking Edge, Spoils of Battle, Summon Forth, The Smith, Wrought in War,
//! and Sword Sage L0/L1. The
//! 30 generated carriers and their exact operands are pinned below so future
//! primitives can widen this family only deliberately.

use super::StepCtx;
use crate::catalog::{CardIdentity, CardSpec, Catalog, CompiledArg};
use crate::content_tables::{Arg, CardRow};
use crate::engine::EngineRefusal;
use crate::engine::cards::{
    FrozenPhysicalCardMove, apply_sword_sage_blade_delta, inject_generated_sovereign_blade,
    live_cards_need_exact_piles, move_frozen_physical_cards_to_bottom,
};
use crate::engine::damage::{
    apply_card_monster_debuff, gain_powered_card_block, note_power, player_attack_all_from_card,
    player_attack_from_card,
};
#[cfg(test)]
use crate::hooks::HookCategory;
use crate::hooks::HookEvent;
use crate::hot::{CARD_FLAG_SOVEREIGN_BLADE_STATE, HotState, MiseryToken, PileId};
use crate::ids::{CardId, PowerId, StepKind, StepWord};
use crate::powers::SlotWire;

#[cfg(test)]
thread_local! {
    static FORGE_WRITER_TRACE: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[cfg(test)]
fn trace_forge_writer(step: &'static str) {
    FORGE_WRITER_TRACE.with(|trace| trace.borrow_mut().push(step));
}

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::ForgeFamilyExact,
    StepKind::Furnace,
    StepKind::HammerTimeExact,
    StepKind::Parry,
    StepKind::SovereignBladeExact,
    StepKind::SwordSage,
];

/// Complete current-build Rust-representable ordinary
/// `AfterSideTurnStart` power census.
///
/// Native has one additional concrete member, Clarity, for which Rust has no
/// `PowerId`; Sandpit belongs to the distinct Late hook. The compact hot
/// carrier records all eighteen PowerIds plus Clarity in acquisition order.
pub(crate) const AFTER_SIDE_TURN_START_POWER_CENSUS: [PowerId; 18] = [
    PowerId::BiasedCognition,
    PowerId::Blur,
    PowerId::Coolant,
    PowerId::Countdown,
    PowerId::DemonForm,
    PowerId::DrawNextTurn,
    PowerId::Feral,
    PowerId::Furnace,
    PowerId::Neurosurge,
    PowerId::NoxiousFumes,
    PowerId::Plating,
    PowerId::Poison,
    PowerId::PrepTime,
    PowerId::Rampart,
    PowerId::Reflect,
    PowerId::ShadowStep,
    PowerId::Slow,
    PowerId::WraithForm,
];

/// Exact canonical Forge-family programs admitted by this slice.
///
/// The shared generated StepKind has ten modes. Beat Into Shape, Wrought in
/// War, Seeking Edge, Conqueror, Refine Blade, and Summon Forth stay in this
/// original matcher. The later cold edge owns The Smith, Big Bang, Bulwark,
/// and Spoils of Battle so widening the family does not move the established
/// hot play region.
pub(crate) fn program_is_supported(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::BeatIntoShape,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("beat_into_shape"), Arg::I(5)]
            }]
        ) | (
            CardId::BeatIntoShape,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("beat_into_shape"), Arg::I(7)]
            }]
        ) | (
            CardId::WroughtInWar,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("wrought_in_war"), Arg::I(7), Arg::I(7)]
            }]
        ) | (
            CardId::WroughtInWar,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("wrought_in_war"), Arg::I(9), Arg::I(9)]
            }]
        ) | (
            CardId::SeekingEdge,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("seeking_edge"), Arg::I(1), Arg::I(7)]
            }]
        ) | (
            CardId::SeekingEdge,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("seeking_edge"), Arg::I(1), Arg::I(11)]
            }]
        ) | (
            CardId::Conqueror,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("conqueror"), Arg::I(3), Arg::I(1)]
            }]
        ) | (
            CardId::Conqueror,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("conqueror"), Arg::I(5), Arg::I(1)]
            }]
        ) | (
            CardId::RefineBlade,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("refine_blade"), Arg::I(8), Arg::I(1)]
            }]
        ) | (
            CardId::RefineBlade,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("refine_blade"), Arg::I(12), Arg::I(1)]
            }]
        ) | (
            CardId::SummonForth,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("summon_forth"), Arg::I(8)]
            }]
        ) | (
            CardId::SummonForth,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("summon_forth"), Arg::I(11)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Complete Forge-writer matcher. The pre-existing rows stay in their original
/// matcher; the four later writers cross one rare cold edge.
#[inline(always)]
pub(crate) fn forge_writer_program_is_supported(row: &CardRow) -> bool {
    if matches!(
        row.id,
        CardId::TheSmith | CardId::BigBang | CardId::Bulwark | CardId::SpoilsOfBattle
    ) {
        return rare_forge_writer_program_is_supported(row);
    }
    program_is_supported(row)
}

#[cold]
#[inline(never)]
fn rare_forge_writer_program_is_supported(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::TheSmith,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("the_smith"), Arg::I(30)]
            }]
        ) | (
            CardId::TheSmith,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("the_smith"), Arg::I(40)]
            }]
        ) | (
            CardId::BigBang,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("big_bang"), Arg::I(5)]
            }]
        ) | (
            CardId::BigBang,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("big_bang"), Arg::I(5)]
            }]
        ) | (
            CardId::Bulwark,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("bulwark"), Arg::I(12), Arg::I(10)]
            }]
        ) | (
            CardId::Bulwark,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("bulwark"), Arg::I(15), Arg::I(13)]
            }]
        ) | (
            CardId::SpoilsOfBattle,
            0,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("spoils_of_battle"), Arg::I(6), Arg::I(2)]
            }]
        ) | (
            CardId::SpoilsOfBattle,
            1,
            [crate::content_tables::Step {
                kind: StepKind::ForgeFamilyExact,
                args: [Arg::S("spoils_of_battle"), Arg::I(9), Arg::I(2)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Exact canonical physical source metadata for rare Forge writers.
#[cfg(test)]
fn forge_writer_spec_is_exact(spec: &CardSpec) -> bool {
    match spec.identity {
        CardIdentity {
            id:
                CardId::SeekingEdge
                | CardId::Conqueror
                | CardId::RefineBlade
                | CardId::SummonForth
                | CardId::TheSmith
                | CardId::BigBang
                | CardId::Bulwark
                | CardId::SpoilsOfBattle,
            upgrade,
            enchantment: None,
        } if upgrade <= 1 => {
            crate::content_tables::card_row(spec.identity.id, upgrade) == Some(spec.row)
                && forge_writer_program_is_supported(spec.row)
        }
        _ => false,
    }
}

/// Exact canonical physical Summon Forth source metadata.
pub(crate) fn summon_forth_spec_is_exact(spec: &CardSpec) -> bool {
    match spec.identity {
        CardIdentity {
            id: CardId::SummonForth,
            upgrade,
            enchantment: None,
        } if upgrade <= 1 => {
            crate::content_tables::card_row(CardId::SummonForth, upgrade) == Some(spec.row)
                && forge_writer_program_is_supported(spec.row)
        }
        _ => false,
    }
}

/// Exact narrow public all-pile collected-AutoPlay exception.
///
/// Body-level authentication is broader, but only Summon Forth and The Smith
/// have independently certified public direct/native routes from Exhaust or
/// Play. Seeking Edge, Conqueror, Refine Blade, Big Bang, Bulwark, and Spoils
/// of Battle retain the generic Sly Hand/Draw/Discard provenance wall.
pub(crate) fn collected_autoplay_all_pile_spec_is_exact(spec: &CardSpec) -> bool {
    match spec.identity {
        CardIdentity {
            id: CardId::SummonForth | CardId::TheSmith,
            upgrade,
            enchantment: None,
        } if upgrade <= 1 => {
            crate::content_tables::card_row(spec.identity.id, upgrade) == Some(spec.row)
                && forge_writer_program_is_supported(spec.row)
        }
        _ => false,
    }
}

/// Whether the catalog contains the complete fixed identity closure shared by
/// Furnace's writer and turn-start Forge reader.
#[cfg(test)]
pub(crate) fn furnace_catalog_is_exact(catalog: &Catalog) -> bool {
    let required = [
        (CardId::Furnace, 0),
        (CardId::Furnace, 1),
        (CardId::SovereignBlade, 0),
        (CardId::SovereignBlade, 1),
    ];
    let required_are_exact = required.into_iter().all(|(id, upgrade)| {
        let identity = CardIdentity {
            id,
            upgrade,
            enchantment: None,
        };
        catalog.atom(&identity).is_some_and(|atom| {
            catalog.spec(atom).is_some_and(|spec| {
                spec.identity == identity
                    && crate::content_tables::card_row(id, upgrade) == Some(spec.row)
            })
        })
    });
    let mut atom = 0;
    let mut catalog_is_closed = true;
    while let Some(spec) = catalog.spec(atom) {
        catalog_is_closed &= required.contains(&(spec.identity.id, spec.identity.upgrade))
            && spec.identity.enchantment.is_none();
        atom += 1;
    }
    required_are_exact
        && catalog_is_closed
        && atom == required.len() as u16
        && !catalog.hooks().any(HookCategory::RelicTemplate)
}

/// Whether the ordinary public catalog contains Furnace's complete fixed
/// generated leaf.
///
/// Unlike [`furnace_catalog_is_exact`], this deliberately permits every other
/// catalog atom. Their row, keyword, physical-state, hook, and recursive
/// generation validity remains owned by the ordinary admission walk; this
/// predicate only authenticates the additional leaf Furnace's cold reader can
/// mint after its source has disappeared.
pub(crate) fn furnace_catalog_closure_is_exact(catalog: &Catalog) -> bool {
    let identity = CardIdentity {
        id: CardId::SovereignBlade,
        upgrade: 0,
        enchantment: None,
    };
    catalog.atom(&identity).is_some_and(|atom| {
        catalog.spec(atom).is_some_and(|spec| {
            spec.identity == identity
                && crate::content_tables::card_row(CardId::SovereignBlade, 0) == Some(spec.row)
        })
    })
}

/// Native `IncreaseSovereignBladeDamage` physical query order. This is not
/// `PileId::ALL`'s wire-name order: each serial `AfterForged` callback belongs
/// to the frozen Hand -> Draw -> Discard -> Exhaust -> Play walk.
const FURNACE_NATIVE_GROWTH_PILES: [PileId; 5] = [
    PileId::Hand,
    PileId::Draw,
    PileId::Discard,
    PileId::Exhaust,
    PileId::Play,
];

/// Exact shared Forge command for already-authenticated Regent callers.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `ForgeCmd::Forge` RVA `0x132bd0` checks ending once, creates one generated
/// level-zero Sovereign Blade iff the live non-Exhaust query is empty, then
/// `IncreaseSovereignBladeDamage` RVA `0x132c24` freezes the include-Exhausted
/// physical list and serially performs `AddDamage` followed by `AfterForged`.
/// The private Furnace subset closes every generated/AfterForged listener, so
/// those callbacks are inert without changing the physical/UID transaction.
pub(crate) fn forge_exact(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if !state.fanouts.hammer_time() {
        return forge_exact_inner(state, catalog, amount, events);
    }
    let mut probe = state.clone();
    forge_exact_inner(&mut probe, catalog, amount, &mut Vec::new())?;
    forge_exact_inner(state, catalog, amount, events)
}

fn forge_exact_inner(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("forge amount"));
    }
    // `ForgeCmd/<Forge>d__2::MoveNext` RVA `0x3ed3bc` returns an empty blade
    // list at `CombatManager::get_IsOverOrEnding` (IL_0025-0032), before the
    // live-blade query (IL_003f) and generated-blade creation (IL_0075), so
    // the gate is the shared IsOverOrEnding projection, not `history.over`
    // (#3112).
    if crate::engine::damage::damage_combat_is_ending(state) {
        return Ok(());
    }
    #[cfg(test)]
    trace_forge_writer("forge");
    let has_live = [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play]
        .into_iter()
        .any(|pile| {
            state.piles.get(pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
        });
    if !has_live {
        inject_generated_sovereign_blade(state, catalog, events)?;
    }
    let amount: i32 = amount
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("sovereign blade damage"))?;
    for pile in FURNACE_NATIVE_GROWTH_PILES {
        for card in state.piles.get(pile).as_slice() {
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            if spec.identity.id != CardId::SovereignBlade {
                continue;
            }
            if card.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE == 0 {
                return Err(EngineRefusal::MalformedArgs(
                    "forge Sovereign Blade payload",
                ));
            }
            let mut instance = state.card_states.get(card.uid);
            instance.damage_growth = instance
                .damage_growth
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("sovereign blade damage"))?;
            state.card_states.set(card.uid, instance);
        }
    }
    if live_cards_need_exact_piles(state, catalog)? {
        state.exact_piles = true;
    }
    if state.fanouts.hammer_time() {
        if state.hp <= 0
            || state.multiplayer_ally_key != 1
            || state.fanouts.multiplayer_ally().key != 1
        {
            return Err(EngineRefusal::MalformedArgs("Hammer Time power state"));
        }
        if state.fanouts.multiplayer_ally().alive {
            crate::engine::allies::forge_remote_player_exact(state, catalog, amount, events)?;
        }
    }
    Ok(())
}

/// Exact Forge transaction for all ten supported Forge-family card modes.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `ForgeCmd::Forge` RVA `0x132bd0` creates a level-zero Sovereign Blade when
/// the live non-Exhaust query is empty; `IncreaseSovereignBladeDamage` RVA
/// `0x132c24` freezes the include-Exhausted list, calls `AddDamage`, then
/// `AfterForged` for each card. `SeekingEdgePower` Type/StackType RVAs
/// `0xa72b4`/`0xa72b7` are Buff/Unique; canonical Forge is 7 at RVA `0xea7b7`
/// and upgrade adds 4 at RVA `0xea80f`; `SeekingEdge/<OnPlay>d__5::MoveNext`
/// RVA `0x3b99fc` applies at IL `0x00c9` before Forge at IL `0x0140`.
/// `ConquerorPower` Type/StackType RVAs `0xa08a2`/`0xa08a5` are
/// Debuff/Intensity; canonical Forge is 3 at RVA `0xdba93` and upgrade adds 2
/// at RVA `0xdbafb`; `Conqueror/<OnPlay>d__5::MoveNext` RVA `0x393f3c`
/// performs Forge at IL `0x00bb..0x0131` before Apply(1) at
/// `0x0132..0x01af`. `RefineBlade::get_CanonicalVars` RVA `0xe939c`
/// supplies Forge 8 and Energy 1; `OnUpgrade` RVA `0xe940f` adds 4 only to
/// Forge. `RefineBlade/<OnPlay>d__5::MoveNext` RVA `0x3b65ac` awaits Forge at
/// IL `0x00a6..0x011c`, then an ending-gated EnergyNextTurnPower Apply(1) at
/// IL `0x011d..0x01a5`. `BeatIntoShape/<OnPlay>d__6::MoveNext` RVA
/// `0x38bebc` attacks before Forge using the pre-attack owner damage-history
/// count; `WroughtInWar/<OnPlay>d__5::MoveNext` RVA `0x3c7be4` likewise
/// attacks before Forge. `TheSmith::get_CanonicalStarCost` RVA `0xeea2e`
/// returns fixed 4, `get_CanonicalVars` RVA `0xeea31` constructs Forge 30,
/// and `OnUpgrade` RVA `0xeea8b` adds 10 only to Forge. Its
/// `<OnPlay>d__7::MoveNext` RVA `0x3c319c` reads that Forge amount at IL
/// `0x009e..0x00ae` and calls `ForgeCmd::Forge` at IL `0x00b3..0x00ba`.
/// `BigBang::get_CanonicalVars` RVA `0xd8e5c` constructs Draw 1, Energy 1,
/// Stars 1, and Forge 5; `OnUpgrade` RVA `0xd8efb` changes only Innate. Its
/// `<OnPlay>d__7::MoveNext` RVA `0x38c648` serially awaits Draw, GainStars,
/// GainEnergy, then Forge. `Bulwark::get_CanonicalVars` RVA `0xda30a`
/// constructs Block 12 and Forge 10 and `OnUpgrade` RVA `0xda383` adds 3 to
/// both; `<OnPlay>d__7::MoveNext` RVA `0x38ff98` awaits powered GainBlock
/// before Forge. `SpoilsOfBattle::get_CanonicalVars` RVA `0xec898` constructs
/// Forge 6 and Draw 2 and `OnUpgrade` RVA `0xec90b` adds 3 only to Forge;
/// `<OnPlay>d__5::MoveNext` RVA `0x3be53c` awaits Forge before Draw.
/// Python `_run_steps_inner` (frozen, deleted #2827), `_forge_commit`,
/// `forge`, and `_summon_forth_exact` are the executable oracle
/// for the shared Forge and frozen-pile-move composition. Ordinary direct
/// AutoPlay's `_start_auto_card_frame` records zero Star and
/// Energy spend without an override; Whispering Earring is a distinct path
/// that spends at `_spend_card_resources` before passing the
/// zero-spent-field override in `_whispering_earring_iteration`.
pub(crate) fn forge_family_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !forge_writer_program_is_supported(ctx.spec.row)
        || ctx.catalog.steps(ctx.spec).len() != 1
        || ctx.catalog.args(ctx.catalog.steps(ctx.spec)[0].args) != ctx.args
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("forge_family_exact"));
    }
    let target = match ctx.spec.identity.id {
        CardId::SeekingEdge
        | CardId::RefineBlade
        | CardId::SummonForth
        | CardId::TheSmith
        | CardId::BigBang
        | CardId::Bulwark
        | CardId::SpoilsOfBattle
            if ctx.target.is_none() =>
        {
            None
        }
        CardId::Conqueror | CardId::BeatIntoShape | CardId::WroughtInWar => Some(
            ctx.target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?,
        ),
        _ => return Err(EngineRefusal::MalformedArgs("forge_family_exact target")),
    };
    let all_piles = matches!(
        ctx.spec.identity.id,
        CardId::SeekingEdge
            | CardId::Conqueror
            | CardId::RefineBlade
            | CardId::SummonForth
            | CardId::TheSmith
            | CardId::BigBang
            | CardId::Bulwark
            | CardId::SpoilsOfBattle
    );
    let mut source = None;
    for pile in PileId::ALL {
        if !all_piles && pile != PileId::Play {
            continue;
        }
        for card in ctx
            .state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .filter(|card| card.uid == ctx.source_uid)
        {
            if source.replace(*card).is_some() {
                return Err(EngineRefusal::MalformedArgs("forge_family_exact source"));
            }
        }
    }
    let Some(source) = source else {
        return Err(EngineRefusal::MalformedArgs("forge_family_exact source"));
    };
    if ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity
        != ctx.spec.identity
    {
        return Err(EngineRefusal::MalformedArgs("forge_family_exact source"));
    }

    fn forge_writer_body(
        ctx: &mut StepCtx<'_>,
        target: Option<usize>,
    ) -> Result<(), EngineRefusal> {
        match ctx.args {
            [
                CompiledArg::Word(StepWord::SeekingEdge),
                CompiledArg::I(1),
                CompiledArg::I(forge_amount),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                match ctx.state.powers.get(PowerId::SeekingEdge) {
                    None => {
                        #[cfg(test)]
                        trace_forge_writer("seeking_power_changed");
                        ctx.state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
                        note_power(
                            ctx.events,
                            crate::engine::Subject::Player,
                            PowerId::SeekingEdge,
                            1,
                        );
                    }
                    Some(slot) if slot.wire == SlotWire::Int && slot.value == 1 => {}
                    Some(_) => {
                        return Err(EngineRefusal::MalformedArgs("Seeking Edge power state"));
                    }
                }
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::Conqueror),
                CompiledArg::I(forge_amount),
                CompiledArg::I(1),
            ] => {
                let target = target.ok_or(EngineRefusal::TargetMismatch { required: true })?;
                let frozen = ctx
                    .state
                    .monsters
                    .get(target)
                    .map(|monster| (monster.slot, monster.uid))
                    .ok_or(EngineRefusal::TargetMismatch { required: true })?;
                if !matches!(
                    crate::engine::play::active_card_current_context(ctx.source_uid),
                    Some((Some(_), Some(identity))) if identity == frozen
                ) {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)?;
                if ctx
                    .state
                    .monsters
                    .get(target)
                    .map(|monster| (monster.slot, monster.uid))
                    != Some(frozen)
                {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
                #[cfg(test)]
                trace_forge_writer("conqueror_power_changed");
                apply_card_monster_debuff(
                    ctx.state,
                    target,
                    PowerId::Conqueror,
                    MiseryToken::Conqueror,
                    1,
                    ctx.events,
                )
            }
            [
                CompiledArg::Word(StepWord::RefineBlade),
                CompiledArg::I(forge_amount),
                CompiledArg::I(1),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)?;
                // `RefineBlade/<OnPlay>d__5` `0x3b65ac` IL_014b is
                // `PowerCmd.Apply<EnergyNextTurnPower>` (MethodDef
                // `0x0600560d`), whose ``<Apply>d__1`1`` `0x3ef988` returns
                // at `CombatManager::get_IsEnding` (IL_0020-0034): the
                // IsEnding projection, not `history.over` (#3218).
                if crate::engine::damage::damage_combat_is_ending(ctx.state) {
                    return Ok(());
                }
                #[cfg(test)]
                trace_forge_writer("refine_energy_next_turn");
                crate::engine::damage::apply_card_energy_next_turn(ctx.state, 1, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::SummonForth),
                CompiledArg::I(forge_amount),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                let mut frozen = Vec::new();
                for pile in [PileId::Hand, PileId::Discard, PileId::Exhaust, PileId::Play] {
                    for (source_index, card) in ctx
                        .state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .copied()
                        .enumerate()
                    {
                        let spec = ctx
                            .catalog
                            .spec(card.atom)
                            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                        if matches!(spec.identity.id, CardId::SovereignBlade) {
                            frozen.push(FrozenPhysicalCardMove {
                                source_pile: pile,
                                source_index,
                                card,
                                instance: ctx.state.card_states.get(card.uid),
                            });
                        }
                    }
                }
                move_frozen_physical_cards_to_bottom(
                    ctx.state,
                    ctx.catalog,
                    &frozen,
                    PileId::Draw,
                    ctx.events,
                )?;
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::TheSmith),
                CompiledArg::I(forge_amount),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::BigBang),
                CompiledArg::I(forge_amount),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                // BigBang MoveNext 0x38c648 serially awaits Draw, GainStars,
                // GainEnergy, then Forge. Under a CardPlay frame the Draw is
                // an owned tail: a Hellraiser selection or Stratagem
                // reshuffle can suspend it, and the Stars/Energy/Forge suffix
                // resumes after the Draw returns (#2665).
                if matches!(
                    ctx.state.frames.top(),
                    Some(crate::frame::Frame::CardPlay { .. })
                ) {
                    #[cfg(test)]
                    trace_forge_writer("big_bang_draw");
                    if crate::engine::play::draw_cardplay_owned_tail(ctx, 1)? {
                        return Ok(());
                    }
                    return apply_big_bang_tail(ctx.state, ctx.catalog, *forge_amount, ctx.events);
                }
                #[cfg(test)]
                trace_forge_writer("big_bang_draw");
                crate::engine::draw::draw_cards(
                    ctx.state,
                    ctx.catalog,
                    1,
                    crate::engine::draw::DrawSource::Command,
                    ctx.events,
                )?;
                apply_big_bang_tail(ctx.state, ctx.catalog, *forge_amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::Bulwark),
                CompiledArg::I(block_amount),
                CompiledArg::I(forge_amount),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                #[cfg(test)]
                trace_forge_writer("bulwark_block");
                gain_powered_card_block(
                    ctx.state,
                    ctx.catalog,
                    ctx.spec,
                    *block_amount,
                    ctx.events,
                )?;
                crate::engine::fire_hook(
                    ctx.catalog,
                    HookEvent::AfterBlockGained,
                    ctx.state,
                    ctx.events,
                )?;
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::SpoilsOfBattle),
                CompiledArg::I(forge_amount),
                CompiledArg::I(2),
            ] => {
                if target.is_some() {
                    return Err(EngineRefusal::TargetMismatch { required: false });
                }
                forge_exact(ctx.state, ctx.catalog, *forge_amount, ctx.events)?;
                #[cfg(test)]
                trace_forge_writer("spoils_draw");
                crate::engine::play::draw_cardplay_no_result(ctx, 2)
            }
            [
                CompiledArg::Word(StepWord::WroughtInWar),
                CompiledArg::I(damage),
                CompiledArg::I(amount),
            ] => {
                let target = target.ok_or(EngineRefusal::TargetMismatch { required: true })?;
                player_attack_from_card(
                    ctx.state,
                    (ctx.catalog, ctx.spec, ctx.source_uid),
                    &[target],
                    *damage,
                    1,
                    ctx.events,
                )?;
                forge_exact(ctx.state, ctx.catalog, *amount, ctx.events)
            }
            [
                CompiledArg::Word(StepWord::BeatIntoShape),
                CompiledArg::I(base),
            ] => {
                let target = target.ok_or(EngineRefusal::TargetMismatch { required: true })?;
                let monster = ctx
                    .state
                    .monsters
                    .get(target)
                    .ok_or(EngineRefusal::TargetMismatch { required: true })?;
                let prior = monster.owner_powered_damage_results_this_turn;
                if prior < 0 {
                    return Err(EngineRefusal::MalformedArgs("beat_into_shape history"));
                }
                let amount = base
                    .checked_mul(1 + i64::from(prior))
                    .ok_or(EngineRefusal::CounterOverflow("beat_into_shape forge"))?;
                player_attack_from_card(
                    ctx.state,
                    (ctx.catalog, ctx.spec, ctx.source_uid),
                    &[target],
                    *base,
                    1,
                    ctx.events,
                )?;
                forge_exact(ctx.state, ctx.catalog, amount, ctx.events)
            }
            _ => Err(EngineRefusal::MalformedArgs("forge_family_exact")),
        }
    }

    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    let mut probe_ctx = StepCtx {
        state: &mut probe,
        catalog: ctx.catalog,
        spec: ctx.spec,
        source_uid: ctx.source_uid,
        target,
        selection: None,
        x_value: 0,
        args: ctx.args,
        events: &mut probe_events,
    };
    crate::engine::play::rehearse_preserving_active_plays(|| {
        forge_writer_body(&mut probe_ctx, target)
    })?;
    forge_writer_body(ctx, target)
}

/// BigBang's awaited Draw/Stars/Energy/Forge program (#2665).
///
/// `BigBang::get_CanonicalVars` constructs Draw 1, Energy 1, Stars 1, Forge
/// 5 and `OnUpgrade` changes only Innate, so both levels share this program.
/// The source row pins the forge-writer word; the compiled operands pin the
/// same word plus Forge 5.
fn big_bang_program(catalog: &Catalog, spec: &CardSpec) -> Option<i64> {
    if !matches!(
        (spec.identity.id, spec.identity.upgrade),
        (CardId::BigBang, 0 | 1)
    ) {
        return None;
    }
    if crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade) != Some(spec.row) {
        return None;
    }
    let [step] = catalog.steps(spec) else {
        return None;
    };
    (step.kind == StepKind::ForgeFamilyExact
        && catalog.args(step.args) == [CompiledArg::Word(StepWord::BigBang), CompiledArg::I(5)])
    .then_some(5)
}

/// The Stars/Energy/Forge suffix after BigBang's Draw returns. Each command
/// carries its own ending gate (a terminal Draw suppresses Stars, Energy,
/// and Forge alike), so the tail is identical inline and on resume.
fn apply_big_bang_tail(
    state: &mut HotState,
    catalog: &Catalog,
    forge_amount: i64,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    #[cfg(test)]
    trace_forge_writer("big_bang_stars");
    crate::engine::play::gain_stars(state, catalog, 1, events)?;
    #[cfg(test)]
    trace_forge_writer("big_bang_energy");
    crate::engine::allies::gain_energy(state, 0, 1)?;
    forge_exact(state, catalog, forge_amount, events)
}

pub(crate) fn resume_big_bang_after_draw(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let forge_amount =
        big_bang_program(catalog, spec).ok_or(EngineRefusal::ContinuationNotModeled)?;
    apply_big_bang_tail(state, catalog, forge_amount, events)
}

/// `furnace` — exact canonical Furnace power application.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) stacks 5/7 on the live FurnacePower.
/// `_run_after_side_turn_start_power_order` reads the amount after Hand draw and
/// invokes the complete Forge transaction once each player turn.
///
/// The public play wrapper rehearses the complete replay series before its
/// shared spend/move prefix. The generalized side-start carrier records
/// Furnace among all ordinary peers, and the turn wrapper owns the whole late
/// hand-draw/enemy/Forge transaction.
pub(crate) fn furnace(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    furnace_foundation_exact(ctx)
}

pub(crate) fn furnace_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::Furnace,
            0,
            [crate::content_tables::Step {
                kind: StepKind::Furnace,
                args: [Arg::I(5)]
            }]
        ) | (
            CardId::Furnace,
            1,
            [crate::content_tables::Step {
                kind: StepKind::Furnace,
                args: [Arg::I(7)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Retained test oracle for the original deliberately narrow Furnace
/// foundation. Public admission now uses [`furnace_public_state_is_exact`]
/// plus its live-source/catalog gates; this stricter predicate remains only to
/// prove that widening did not weaken the old closed singleton subset.
#[cfg(test)]
pub(crate) fn furnace_private_state_is_exact(state: &HotState) -> bool {
    let order = state.fanouts.after_side_turn_start_order();
    let type_one_listener_order_is_exact =
        crate::engine::damage::player_type_one_listener_order_is_exact(state);
    let power_is_exact = match state.powers.as_slice() {
        [] => order.is_empty(),
        [slot]
            if slot.key == crate::ids::PowerId::Furnace
                && slot.wire == SlotWire::Int
                && slot.value > 0 =>
        {
            order == [crate::hot::AfterSideTurnStartToken::Furnace]
        }
        _ => false,
    };
    let physical_uids_are_exact = PileId::ALL.into_iter().all(|pile| {
        state.piles.get(pile).as_slice().iter().all(|card| {
            card.uid < state.next_card_uid
                && PileId::ALL
                    .into_iter()
                    .flat_map(|candidate| state.piles.get(candidate).as_slice())
                    .filter(|candidate| candidate.uid == card.uid)
                    .count()
                    == 1
        })
    });
    power_is_exact
        && type_one_listener_order_is_exact
        // Furnace's private quotient is closed over every hidden shared-gate
        // carrier too: an orphan Monologue ledger/registration must not pass
        // merely because its public power slot is absent.
        && state.fanouts.monologue_strength_applied() == 0
        && !state.fanouts.monologue_hooks_are_registered()
        && state.fanouts.monologue_hook_flags_are_exact()
        && !state.fanouts.ruined_helmet_used()
        && state.multiplayer_ally_key == 0
        && !state.ringing()
        && state.exact_piles
        && physical_uids_are_exact
        && state.fanouts.after_card_drawn_order().is_empty()
        && state.fanouts.after_card_exhausted_order().is_empty()
        && state.fanouts.before_hand_draw_order().is_empty()
        && state.fanouts.after_damage_given_order().is_empty()
        && state.fanouts.after_block_gained_order().is_empty()
        && state.fanouts.after_block_cleared_order().is_empty()
        && state.fanouts.before_side_turn_end_order().is_empty()
        && state.fanouts.star_energy_reset_order().is_empty()
        && state.fanouts.local_generated_power_order().is_empty()
        && state.fanouts.result_location_power_order().is_empty()
}

/// Authenticate the public Furnace-owned state without excluding unrelated
/// admitted mechanics on disjoint hooks.
///
/// A positive Furnace is a solo positive Int amount paired with its token in
/// the complete ordinary acquisition order. Same-hook peers are represented
/// by that order; Furnace's distinct owner/source and Forge walls remain
/// independent admission requirements.
pub(crate) fn furnace_public_state_is_exact(state: &HotState) -> bool {
    if state.multiplayer_ally_key != 0 {
        return false;
    }
    let Some(order) = state.after_side_turn_start_power_order() else {
        return false;
    };
    let registered = order.contains(&crate::hot::AfterSideTurnStartToken::Furnace);
    match state.powers.get(PowerId::Furnace) {
        None => !registered,
        Some(slot) => slot.wire == SlotWire::Int && slot.value > 0 && registered,
    }
}

/// Preflight every Furnace-observable Forge input before the turn rehearsal
/// starts. This is deliberately stronger than relying on the outer rollback:
/// malformed payloads, catalog atoms, counters, and growth arithmetic are
/// rejected before even the cloned hand-draw/turn prefix runs.
pub(crate) fn furnace_turn_preflight_is_exact(
    state: &HotState,
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    if state.piles.get(PileId::Hand).len() > crate::engine::draw::MAX_CARDS_IN_HAND {
        return Err(EngineRefusal::MalformedArgs("furnace hand size"));
    }
    let amount = state
        .powers
        .get(crate::ids::PowerId::Furnace)
        .filter(|slot| slot.wire == SlotWire::Int && slot.value > 0)
        .ok_or(EngineRefusal::MalformedArgs("furnace live amount"))?
        .value;
    let mut has_live_blade = false;
    for pile in PileId::ALL {
        for card in state.piles.get(pile).as_slice() {
            if card.uid >= state.next_card_uid
                || PileId::ALL
                    .into_iter()
                    .flat_map(|candidate| state.piles.get(candidate).as_slice())
                    .filter(|candidate| candidate.uid == card.uid)
                    .count()
                    != 1
            {
                return Err(EngineRefusal::MalformedArgs("furnace physical uid"));
            }
            let spec = catalog
                .spec(card.atom)
                .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
            let instance = state.card_states.get(card.uid);
            match spec.identity.id {
                CardId::Furnace => {}
                CardId::SovereignBlade => {
                    if card.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE == 0
                        || (instance.base_replay_count().is_some()
                            && card.flags & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE == 0)
                        || instance.damage_growth < 0
                    {
                        return Err(EngineRefusal::MalformedArgs(
                            "forge Sovereign Blade payload",
                        ));
                    }
                    if instance.damage_growth.checked_add(amount).is_none() {
                        return Err(EngineRefusal::CounterOverflow("sovereign blade damage"));
                    }
                    has_live_blade |= pile != PileId::Exhaust;
                }
                _ => {}
            }
        }
    }
    // Validate occupied Sovereign Blade rows here because Forge reads them.
    // Ordinary card-state ownership remains the public admission boundary's
    // responsibility; this private reader must not reject unrelated admitted
    // per-card mechanics. The preceding physical walk independently
    // authenticates UID uniqueness and allocator monotonicity.
    for (uid, instance) in state.card_states.as_slice() {
        let mut owners = PileId::ALL.into_iter().flat_map(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .filter(move |card| card.uid == *uid)
        });
        let Some(owner) = owners.next() else { continue };
        let duplicate = owners.next().is_some();
        let owner_id = catalog.spec(owner.atom).map(|spec| spec.identity.id);
        if duplicate || (owner_id == Some(CardId::SovereignBlade) && instance.damage_growth < 0) {
            return Err(EngineRefusal::MalformedArgs("furnace orphan card state"));
        }
    }
    if state.history.owner_generated_cards_combat < 0 || state.next_generated_hook_uid < 0 {
        return Err(EngineRefusal::MalformedArgs("furnace generated counters"));
    }
    if !has_live_blade {
        state
            .next_card_uid
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("next_card_uid"))?;
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
        10_i32
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("sovereign blade damage"))?;
    }
    Ok(())
}

/// Exact canonical Furnace writer shared by public dispatch and focused tests.
///
/// `Furnace/<OnPlay>d__5::MoveNext` RVA `0x3a0718` has no card-level ending
/// check; IL_00cc is `PowerCmd.Apply<FurnacePower>` (MethodDef `0x0600560d`),
/// whose ``<Apply>d__1`1::MoveNext`` RVA `0x3ef988` returns at
/// `CombatManager::get_IsEnding` (IL_0020-0034) before the power exists. After
/// its source and arguments authenticate, the writer is a no-op on the
/// IsEnding projection; it no longer refuses on `history.over` (#3218).
///
/// Pile order is not read (#3221): `Furnace/<OnPlay>d__5::MoveNext` RVA
/// `0x3a0718` triggers the anim (IL_0044) and applies `FurnacePower` with the
/// Forge BaseValue (IL_00b5..IL_00cc), so inexact piles are accepted and left
/// inexact.
pub(crate) fn furnace_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !furnace_program_is_exact(ctx.spec.row)
        || ctx.catalog.steps(ctx.spec).len() != 1
        || ctx.catalog.args(ctx.catalog.steps(ctx.spec)[0].args) != ctx.args
        || ctx.selection.is_some()
        || ctx.target.is_some()
        || ctx.x_value != 0
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || !furnace_catalog_closure_is_exact(ctx.catalog)
        || !furnace_public_state_is_exact(ctx.state)
        || !crate::engine::play::apotheosis_body_context_is_exact(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
        )
    {
        return Err(EngineRefusal::MalformedArgs("furnace foundation"));
    }
    let mut sources = ctx
        .state
        .piles
        .get(PileId::Play)
        .as_slice()
        .iter()
        .filter(|card| card.uid == ctx.source_uid)
        .copied();
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::MalformedArgs("furnace physical source"));
    };
    if sources.next().is_some()
        || ctx.catalog.spec(source.atom).map(|spec| spec.identity) != Some(ctx.spec.identity)
    {
        return Err(EngineRefusal::MalformedArgs("furnace physical source"));
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount > 0 => {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("furnace amount"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("furnace foundation")),
    };
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }

    fn apply(
        state: &mut HotState,
        amount: i32,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let prior = state.powers.value(crate::ids::PowerId::Furnace);
        let updated = prior
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow("furnace amount"))?;
        if prior == 0
            && !state
                .fanouts
                .register_after_side_turn_start(crate::ids::PowerId::Furnace)
        {
            return Err(EngineRefusal::PowerOrderNotModeled("AfterSideTurnStart"));
        }
        state
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, updated);
        crate::engine::damage::note_power(
            events,
            crate::engine::Subject::Player,
            crate::ids::PowerId::Furnace,
            updated,
        );
        Ok(())
    }

    let mut probe = ctx.state.clone();
    let mut probe_events = Vec::new();
    apply(&mut probe, amount, &mut probe_events)?;
    if !furnace_public_state_is_exact(&probe) {
        return Err(EngineRefusal::MalformedArgs("furnace foundation result"));
    }
    apply(ctx.state, amount, ctx.events)
}

/// Exact Hammer Time unique-power writer.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `HammerTimePower` Type/StackType RVAs `0xa322b`/`0xa322e` are Buff/Unique.
/// `HammerTimePower/<AfterForge>d__10::MoveNext` RVA `0x33bbd8` returns for a
/// HammerTimePower source, requires the local owner as forger, freezes living
/// remote Players in native order, and serially awaits `ForgeCmd::Forge` with
/// this power as source. `HammerTime/<OnPlay>d__5::MoveNext` installs amount
/// one. Python `_validate_batch299_hammer_time_entry_spec` (frozen, deleted #2827) and
/// `_run_steps_inner` pin both canonical rows and the unique writer;
/// `_validated_hammer_time_after_forge_recipients` and
/// `_forge_remote_player` pin the fail-closed nested Forge suffix.
pub(crate) fn hammer_time_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !hammer_time_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::HammerTimeExact
                && ctx.catalog.args(step.args) == ctx.args)
        || !matches!(ctx.args, [CompiledArg::I(1)])
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || ctx.state.hp <= 0
        || ctx.state.multiplayer_ally_key != 1
        || ctx.state.fanouts.multiplayer_ally().key != 1
        || !ctx.state.fanouts.multiplayer_ally().alive
        || !furnace_catalog_closure_is_exact(ctx.catalog)
        || !crate::engine::play::apotheosis_body_context_is_exact(
            ctx.state,
            ctx.catalog,
            ctx.source_uid,
        )
    {
        return Err(EngineRefusal::MalformedArgs("hammer_time_exact"));
    }
    let mut sources = ctx
        .state
        .piles
        .get(PileId::Play)
        .as_slice()
        .iter()
        .filter(|card| card.uid == ctx.source_uid)
        .copied();
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::MalformedArgs(
            "hammer_time_exact physical source",
        ));
    };
    if sources.next().is_some() || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "hammer_time_exact physical source",
        ));
    }
    // `HammerTime/<OnPlay>d__5` `0x3a3458` IL_00c1 is
    // `PowerCmd.Apply<HammerTimePower>` (MethodDef `0x0600560d`), whose
    // ``<Apply>d__1`1`` `0x3ef988` returns at `CombatManager::get_IsEnding`
    // (IL_0020-0034): a no-op on the IsEnding projection after the source
    // authenticates, where this writer formerly refused on `history.over`
    // (#3218).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    // StackType.Unique: replays preserve amount one rather than accumulating.
    ctx.state.fanouts.set_hammer_time(true);
    Ok(())
}

pub(crate) fn hammer_time_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::HammerTime,
            0 | 1,
            [crate::content_tables::Step {
                kind: StepKind::HammerTimeExact,
                args: [Arg::I(1)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Exact canonical Parry writer.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Parry::get_CanonicalVars` RVA `0xe73ad` starts `ParryPower` at 10 and
/// `OnUpgrade` RVA `0xe740b` adds 4. `<OnPlay>d__5::MoveNext` RVA `0x3b17e4`
/// applies that live additive power at IL `0x00d1`. Python
/// `_run_steps_inner` (frozen, deleted #2827) authenticates exactly those two rows and
/// adds only while combat is not ending.
pub(crate) fn parry(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !parry_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::Parry
                && ctx.catalog.args(step.args) == ctx.args)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
        || !parry_public_state_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("parry"));
    }
    let mut sources = ctx
        .state
        .piles
        .get(PileId::Play)
        .as_slice()
        .iter()
        .filter(|card| card.uid == ctx.source_uid)
        .copied();
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::MalformedArgs("parry physical source"));
    };
    if sources.next().is_some() || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("parry physical source"));
    }
    // IL_00d1 is `PowerCmd.Apply<ParryPower>` (MethodDef `0x0600560d`); its
    // ``<Apply>d__1`1`` `0x3ef988` returns at `CombatManager::get_IsEnding`
    // (IL_0020-0034), so the skip is the IsEnding projection (#3218).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let amount = match ctx.args {
        [CompiledArg::I(amount)] if *amount > 0 => {
            i32::try_from(*amount).map_err(|_| EngineRefusal::CounterOverflow("parry amount"))?
        }
        _ => return Err(EngineRefusal::MalformedArgs("parry")),
    };
    let updated = ctx
        .state
        .powers
        .value(PowerId::Parry)
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("parry amount"))?;
    ctx.state.powers.set(PowerId::Parry, SlotWire::Int, updated);
    note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::Parry,
        updated,
    );
    Ok(())
}

pub(crate) fn parry_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::Parry,
            0,
            [crate::content_tables::Step {
                kind: StepKind::Parry,
                args: [Arg::I(10)]
            }]
        ) | (
            CardId::Parry,
            1,
            [crate::content_tables::Step {
                kind: StepKind::Parry,
                args: [Arg::I(14)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Parry has no listener token or private ledger: its complete canonical state
/// is an absent slot or one positive Int amount in the ordinary sparse power
/// carrier. Zero is canonically elided.
pub(crate) fn parry_public_state_is_exact(state: &HotState) -> bool {
    state
        .powers
        .get(PowerId::Parry)
        .is_none_or(|slot| slot.wire == SlotWire::Int && slot.value > 0)
}

/// `sovereign_blade_exact` — exact represented Sovereign Blade attack.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `SovereignBlade/<OnPlay>d__25::MoveNext` RVA `0x3bd568` emits one
/// AttackCommand over `TargetingAllOpponents` at IL `0x00ee..0x0129` or the
/// captured target at `0x0130..0x01b7`, then resolves the attack before the
/// Parry-powered block suffix. The shared dynamic-target helper separately
/// pins `get_TargetType` RVA `0xebed7` and `HasSeekingEdge` RVA `0xec17e`.
///
/// Python: `sovereign_blade_state` (frozen, deleted #2827) validates physical slot 6 as
/// `(SOVEREIGN_BLADE, damage, repeats)`. `_run_steps_inner` reads that
/// live payload, chooses one target or all living targets from Seeking Edge,
/// attacks with the stored damage/repeat count through the Sovereign-only
/// Conqueror multiplier, then gains live Parry block.
///
/// The boundary retains the live nonnegative damage and validates the native
/// fixed repeat count. The body re-reads the shared dynamic target seam: an
/// absent Seeking Edge authenticates one frozen `(slot, uid)`, while the
/// positive unique power allocates one roster-order vector of living monsters
/// and issues one multi-target Attack command. Parry's live positive amount
/// enters the existing powered Block pipeline only after the complete attack.
pub(crate) fn sovereign_blade_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !sovereign_blade_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::SovereignBladeExact
                && ctx.catalog.args(step.args) == ctx.args)
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || !ctx.args.is_empty()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !parry_public_state_is_exact(ctx.state)
    {
        return Err(EngineRefusal::MalformedArgs("sovereign_blade_exact"));
    }
    let effective_target_type = crate::engine::play::effective_target_type(
        ctx.state,
        ctx.catalog,
        ctx.spec,
        ctx.source_uid,
    )?;
    let mut sources = PileId::ALL.into_iter().flat_map(|pile| {
        ctx.state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .filter(|card| card.uid == ctx.source_uid)
            .copied()
    });
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::MalformedArgs(
            "sovereign_blade_exact physical source",
        ));
    };
    let instance = ctx.state.card_states.get(ctx.source_uid);
    if sources.next().is_some()
        || source.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE == 0
        || ctx.catalog.spec(source.atom) != Some(ctx.spec)
        || instance.damage_growth < 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "sovereign_blade_exact physical source",
        ));
    }
    let damage = instance.damage_growth;
    match effective_target_type {
        crate::catalog::CardTargetType::AnyEnemy => {
            let target = ctx
                .target
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let monster = ctx
                .state
                .monsters
                .get(target)
                .ok_or(EngineRefusal::TargetMismatch { required: true })?;
            let Some((Some(_), Some(frozen_target))) =
                crate::engine::play::active_card_current_context(ctx.source_uid)
            else {
                return Err(EngineRefusal::ContinuationNotModeled);
            };
            if frozen_target != (monster.slot, monster.uid) {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            player_attack_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                &[target],
                i64::from(damage),
                1,
                ctx.events,
            )?;
        }
        crate::catalog::CardTargetType::AllEnemies => {
            if ctx.target.is_some()
                || !matches!(
                    crate::engine::play::active_card_current_context(ctx.source_uid),
                    Some((Some(_), None))
                )
            {
                return Err(EngineRefusal::TargetMismatch { required: false });
            }
            player_attack_all_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                i64::from(damage),
                1,
                ctx.events,
            )?;
        }
        _ => return Err(EngineRefusal::MalformedArgs("sovereign_blade target shape")),
    }
    // Native awaits the entire attack, then calls GetOwnerParryAmount again.
    // A lethal attack therefore reaches the existing GainBlock entry no-op;
    // a live amount is neither snapshotted before damage nor applied per hit.
    let parry = ctx.state.powers.value(PowerId::Parry);
    if parry > 0 {
        gain_powered_card_block(
            ctx.state,
            ctx.catalog,
            ctx.spec,
            i64::from(parry),
            ctx.events,
        )?;
    }
    Ok(())
}

pub(crate) fn sovereign_blade_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::SovereignBlade,
            0 | 1,
            [crate::content_tables::Step {
                kind: StepKind::SovereignBladeExact,
                args: []
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Apply one exact additive Sword Sage power body.
///
/// Python oracle: `_apply_sword_sage_power` (frozen Python, deleted #2827) writes the physical scalar;
/// `_apply_action_impl` and `_start_auto_card_frame` then read it.
/// Current v0.111.0 native authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// Sword Sage L0/L1 construct amount one; `SwordSage::OnPlay` outer RVA
/// `0xedda0` and `<OnPlay>d__5::MoveNext` RVA `0x3c1548` apply it.
/// `SwordSagePower.AfterPowerAmountChanged` RVA `0xa902c`
/// updates every owned physical Sovereign Blade by the exact amount delta.
/// The checked physical rewrite precedes publication of the new scalar.
pub(crate) fn sword_sage(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !sword_sage_program_is_exact(ctx.spec.row)
        || !matches!(ctx.catalog.steps(ctx.spec), [step]
            if step.kind == StepKind::SwordSage
                && ctx.catalog.args(step.args) == ctx.args)
        || !matches!(ctx.args, [CompiledArg::I(1)])
        || !crate::engine::play::body_enchantment_is_exact(ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
        || !matches!(
            crate::engine::play::active_card_current_context(ctx.source_uid),
            Some((Some(_), None))
        )
    {
        return Err(EngineRefusal::MalformedArgs("sword_sage"));
    }
    let Some((source_pile, source_index)) =
        crate::engine::play::unique_live_card_location(ctx.state, ctx.source_uid)?
    else {
        return Err(EngineRefusal::MalformedArgs("sword_sage physical source"));
    };
    let source = ctx.state.piles.get(source_pile).as_slice()[source_index];
    if source_pile == PileId::Exhaust || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs("sword_sage physical source"));
    }
    // `SwordSage/<OnPlay>d__5` `0x3c1548` IL_00d1 is
    // `PowerCmd.Apply<SwordSagePower>` (MethodDef `0x0600560d`); its
    // ``<Apply>d__1`1`` `0x3ef988` returns at `CombatManager::get_IsEnding`
    // (IL_0020-0034) before the power exists, so its blade rewrite
    // (`SwordSagePower::AfterPowerAmountChanged` `0xa902c`) never runs either;
    // the skip is the IsEnding projection (#3218).
    if crate::engine::damage::damage_combat_is_ending(ctx.state) {
        return Ok(());
    }
    let updated = ctx
        .state
        .powers
        .value(PowerId::SwordSage)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Sword Sage amount"))?;
    if !crate::engine::damage::player_type_one_listener_order_is_exact(ctx.state)
        || !ctx
            .state
            .fanouts
            .can_register_after_power_amount_changed(PowerId::SwordSage)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Sword Sage AfterPowerAmountChanged order",
        ));
    }
    apply_sword_sage_blade_delta(ctx.state, ctx.catalog, 1)?;
    assert!(
        ctx.state
            .fanouts
            .register_after_power_amount_changed(PowerId::SwordSage),
        "listener registration was preflighted"
    );
    ctx.state
        .powers
        .set(PowerId::SwordSage, SlotWire::Int, updated);
    note_power(
        ctx.events,
        crate::engine::Subject::Player,
        PowerId::SwordSage,
        updated,
    );
    Ok(())
}

pub(crate) fn sword_sage_program_is_exact(row: &CardRow) -> bool {
    matches!(
        (row.id, row.upgrade, row.steps),
        (
            CardId::SwordSage,
            0 | 1,
            [crate::content_tables::Step {
                kind: StepKind::SwordSage,
                args: [Arg::I(1)]
            }]
        )
    ) && crate::content_tables::card_row(row.id, row.upgrade) == Some(row)
}

/// Remove the live Sword Sage power and reverse its full contribution from
/// every owned Blade, including mutable clones.
#[allow(dead_code)] // Current content has no power-removal source; lifecycle is still exact.
pub(crate) fn remove_sword_sage_power(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    let amount = state.powers.value(PowerId::SwordSage);
    if !crate::engine::damage::player_type_one_listener_order_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Sword Sage AfterPowerAmountChanged order",
        ));
    }
    if amount == 0 {
        return Ok(());
    }
    apply_sword_sage_blade_delta(state, catalog, -amount)?;
    state
        .fanouts
        .unregister_after_power_amount_changed(PowerId::SwordSage);
    state.powers.set(PowerId::SwordSage, SlotWire::Int, 0);
    note_power(
        events,
        crate::engine::Subject::Player,
        PowerId::SwordSage,
        0,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogBuilder;
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CardInstanceState, HotCard, HotMonster, HotState,
        LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, LocalCostModifiers,
        MultiplayerAllyCard, MultiplayerAllyState,
    };
    use crate::ids::{CardId, EnchantmentId, MonsterKind};

    const OWNED: [StepKind; 6] = [
        StepKind::ForgeFamilyExact,
        StepKind::Furnace,
        StepKind::HammerTimeExact,
        StepKind::Parry,
        StepKind::SovereignBladeExact,
        StepKind::SwordSage,
    ];

    #[test]
    fn regent_forge_manifest_claims_hammer_parry_and_the_exact_blade_reader() {
        assert_eq!(
            IMPLEMENTED,
            [
                StepKind::ForgeFamilyExact,
                StepKind::Furnace,
                StepKind::HammerTimeExact,
                StepKind::Parry,
                StepKind::SovereignBladeExact,
                StepKind::SwordSage
            ]
        );
        for kind in OWNED {
            let expected = matches!(
                kind,
                StepKind::ForgeFamilyExact
                    | StepKind::Furnace
                    | StepKind::HammerTimeExact
                    | StepKind::Parry
                    | StepKind::SovereignBladeExact
                    | StepKind::SwordSage
            );
            assert_eq!(IMPLEMENTED.contains(&kind), expected);
            assert_eq!(crate::steps::is_implemented(kind), expected);
            assert_eq!(
                crate::engine::capability_manifest().steps.contains(&kind),
                expected
            );
        }
        assert_eq!(
            FURNACE_NATIVE_GROWTH_PILES,
            [
                PileId::Hand,
                PileId::Draw,
                PileId::Discard,
                PileId::Exhaust,
                PileId::Play,
            ],
            "Forge freezes Sovereign Blades in native physical-query order"
        );
    }

    #[test]
    fn all_thirty_generated_carriers_and_operands_remain_source_derived() {
        let carriers = CARD_ROWS
            .iter()
            .flat_map(|row| {
                row.steps.iter().filter_map(move |step| {
                    OWNED.contains(&step.kind).then_some((
                        row.id,
                        row.upgrade,
                        step.kind,
                        step.args,
                    ))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            carriers,
            vec![
                (
                    CardId::BeatIntoShape,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("beat_into_shape"), Arg::I(5)][..],
                ),
                (
                    CardId::BeatIntoShape,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("beat_into_shape"), Arg::I(7)][..],
                ),
                (
                    CardId::BigBang,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("big_bang"), Arg::I(5)][..],
                ),
                (
                    CardId::BigBang,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("big_bang"), Arg::I(5)][..],
                ),
                (
                    CardId::Bulwark,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("bulwark"), Arg::I(12), Arg::I(10)][..],
                ),
                (
                    CardId::Bulwark,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("bulwark"), Arg::I(15), Arg::I(13)][..],
                ),
                (
                    CardId::Conqueror,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("conqueror"), Arg::I(3), Arg::I(1)][..],
                ),
                (
                    CardId::Conqueror,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("conqueror"), Arg::I(5), Arg::I(1)][..],
                ),
                (CardId::Furnace, 0, StepKind::Furnace, &[Arg::I(5)][..]),
                (CardId::Furnace, 1, StepKind::Furnace, &[Arg::I(7)][..]),
                (
                    CardId::HammerTime,
                    0,
                    StepKind::HammerTimeExact,
                    &[Arg::I(1)][..],
                ),
                (
                    CardId::HammerTime,
                    1,
                    StepKind::HammerTimeExact,
                    &[Arg::I(1)][..],
                ),
                (CardId::Parry, 0, StepKind::Parry, &[Arg::I(10)][..]),
                (CardId::Parry, 1, StepKind::Parry, &[Arg::I(14)][..]),
                (
                    CardId::RefineBlade,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("refine_blade"), Arg::I(8), Arg::I(1)][..],
                ),
                (
                    CardId::RefineBlade,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("refine_blade"), Arg::I(12), Arg::I(1)][..],
                ),
                (
                    CardId::SeekingEdge,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("seeking_edge"), Arg::I(1), Arg::I(7)][..],
                ),
                (
                    CardId::SeekingEdge,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("seeking_edge"), Arg::I(1), Arg::I(11)][..],
                ),
                (
                    CardId::SovereignBlade,
                    0,
                    StepKind::SovereignBladeExact,
                    &[][..],
                ),
                (
                    CardId::SovereignBlade,
                    1,
                    StepKind::SovereignBladeExact,
                    &[][..],
                ),
                (
                    CardId::SpoilsOfBattle,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("spoils_of_battle"), Arg::I(6), Arg::I(2)][..],
                ),
                (
                    CardId::SpoilsOfBattle,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("spoils_of_battle"), Arg::I(9), Arg::I(2)][..],
                ),
                (
                    CardId::SummonForth,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("summon_forth"), Arg::I(8)][..],
                ),
                (
                    CardId::SummonForth,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("summon_forth"), Arg::I(11)][..],
                ),
                (CardId::SwordSage, 0, StepKind::SwordSage, &[Arg::I(1)][..],),
                (CardId::SwordSage, 1, StepKind::SwordSage, &[Arg::I(1)][..],),
                (
                    CardId::TheSmith,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("the_smith"), Arg::I(30)][..],
                ),
                (
                    CardId::TheSmith,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("the_smith"), Arg::I(40)][..],
                ),
                (
                    CardId::WroughtInWar,
                    0,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("wrought_in_war"), Arg::I(7), Arg::I(7)][..],
                ),
                (
                    CardId::WroughtInWar,
                    1,
                    StepKind::ForgeFamilyExact,
                    &[Arg::S("wrought_in_war"), Arg::I(9), Arg::I(9)][..],
                ),
            ]
        );
        let supported = carriers
            .iter()
            .filter(|(id, upgrade, kind, _)| {
                let row = crate::content_tables::card_row(*id, *upgrade).unwrap();
                *kind == StepKind::SovereignBladeExact
                    || *kind == StepKind::Parry && parry_program_is_exact(row)
                    || *kind == StepKind::HammerTimeExact && hammer_time_program_is_exact(row)
                    || *kind == StepKind::SwordSage && sword_sage_program_is_exact(row)
                    || forge_writer_program_is_supported(row)
            })
            .map(|(id, upgrade, _, _)| (*id, *upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            supported,
            [
                (CardId::BeatIntoShape, 0),
                (CardId::BeatIntoShape, 1),
                (CardId::BigBang, 0),
                (CardId::BigBang, 1),
                (CardId::Bulwark, 0),
                (CardId::Bulwark, 1),
                (CardId::Conqueror, 0),
                (CardId::Conqueror, 1),
                (CardId::HammerTime, 0),
                (CardId::HammerTime, 1),
                (CardId::Parry, 0),
                (CardId::Parry, 1),
                (CardId::RefineBlade, 0),
                (CardId::RefineBlade, 1),
                (CardId::SeekingEdge, 0),
                (CardId::SeekingEdge, 1),
                (CardId::SovereignBlade, 0),
                (CardId::SovereignBlade, 1),
                (CardId::SpoilsOfBattle, 0),
                (CardId::SpoilsOfBattle, 1),
                (CardId::SummonForth, 0),
                (CardId::SummonForth, 1),
                (CardId::SwordSage, 0),
                (CardId::SwordSage, 1),
                (CardId::TheSmith, 0),
                (CardId::TheSmith, 1),
                (CardId::WroughtInWar, 0),
                (CardId::WroughtInWar, 1),
            ]
        );
    }

    #[test]
    fn family_program_gate_admits_all_ten_forge_modes_and_blade_rows() {
        let family_rows = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| OWNED.contains(&step.kind)))
            .collect::<Vec<_>>();
        assert_eq!(family_rows.len(), 30);
        assert!(family_rows.iter().all(|row| row.steps.len() == 1));
        let admitted = family_rows
            .iter()
            .filter(|row| {
                row.steps.iter().all(|step| {
                    step.kind == StepKind::SovereignBladeExact
                        || step.kind == StepKind::Parry && parry_program_is_exact(row)
                        || step.kind == StepKind::SwordSage && sword_sage_program_is_exact(row)
                        || step.kind == StepKind::ForgeFamilyExact
                            && forge_writer_program_is_supported(row)
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(admitted.len(), 26);
        assert!(admitted.iter().all(|row| matches!(
            row.id,
            CardId::BeatIntoShape
                | CardId::BigBang
                | CardId::Bulwark
                | CardId::Conqueror
                | CardId::Parry
                | CardId::RefineBlade
                | CardId::SeekingEdge
                | CardId::SovereignBlade
                | CardId::SpoilsOfBattle
                | CardId::SummonForth
                | CardId::SwordSage
                | CardId::TheSmith
                | CardId::WroughtInWar
        ) && row.upgrade <= 1));
        let mut malformed = *crate::content_tables::card_row(CardId::Parry, 0).unwrap();
        malformed.upgrade = 2;
        assert!(!parry_program_is_exact(&malformed));
        malformed = *crate::content_tables::card_row(CardId::Parry, 0).unwrap();
        malformed.steps = crate::content_tables::card_row(CardId::SovereignBlade, 0)
            .unwrap()
            .steps;
        assert!(!parry_program_is_exact(&malformed));

        let canonical_refine = crate::content_tables::card_row(CardId::RefineBlade, 0).unwrap();
        assert_eq!(canonical_refine.cost, 1);
        assert!(canonical_refine.is_skill);
        assert!(!canonical_refine.targeted);
        assert_eq!(canonical_refine.target_type, "Self");
        for drift in 0..4 {
            let mut malformed = *canonical_refine;
            match drift {
                0 => malformed.cost = 0,
                1 => malformed.is_skill = false,
                2 => malformed.targeted = true,
                3 => malformed.target_type = "AnyEnemy",
                _ => unreachable!(),
            }
            assert!(!forge_writer_program_is_supported(&malformed));
        }
        for upgrade in 0..=1 {
            let canonical = crate::content_tables::card_row(CardId::SummonForth, upgrade).unwrap();
            assert_eq!(canonical.cost, 1);
            assert!(canonical.is_skill);
            assert!(!canonical.targeted);
            assert_eq!(canonical.target_type, "Self");
            assert!(!canonical.selects && !canonical.x_cost);
            let mut malformed = *canonical;
            malformed.cost = 0;
            assert!(!program_is_supported(&malformed));
            malformed = *canonical;
            malformed.targeted = true;
            assert!(!program_is_supported(&malformed));

            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::SummonForth,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            assert!(summon_forth_spec_is_exact(&spec));
            let mut enchanted = spec;
            enchanted.identity.enchantment = Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Adroit,
                amount: 1,
            });
            assert!(!summon_forth_spec_is_exact(&enchanted));
            let mut alien_row = spec;
            alien_row.row = crate::content_tables::card_row(CardId::RefineBlade, upgrade).unwrap();
            assert!(!summon_forth_spec_is_exact(&alien_row));
        }

        for (upgrade, forge) in [(0, 30), (1, 40)] {
            let canonical = crate::content_tables::card_row(CardId::TheSmith, upgrade).unwrap();
            assert_eq!(canonical.cost, 1);
            assert_eq!(canonical.star_cost, 4);
            assert!(canonical.is_skill && !canonical.is_power);
            assert!(!canonical.targeted && matches!(canonical.target_type, "Self"));
            assert!(!canonical.selects && !canonical.x_cost && !canonical.star_x);
            assert!(matches!(
                canonical.steps,
                [crate::content_tables::Step {
                    kind: StepKind::ForgeFamilyExact,
                    args: [Arg::S("the_smith"), Arg::I(amount)],
                }] if *amount == forge
            ));
            for drift in 0..8 {
                let mut malformed = *canonical;
                match drift {
                    0 => malformed.cost = 0,
                    1 => malformed.star_cost = 3,
                    2 => malformed.is_skill = false,
                    3 => malformed.is_power = true,
                    4 => malformed.targeted = true,
                    5 => malformed.target_type = "AnyEnemy",
                    6 => malformed.selects = true,
                    7 => malformed.x_cost = true,
                    _ => unreachable!(),
                }
                assert!(!forge_writer_program_is_supported(&malformed));
            }

            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::TheSmith,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let spec = *catalog.spec(atom).unwrap();
            assert!(forge_writer_spec_is_exact(&spec));
            let mut enchanted = spec;
            enchanted.identity.enchantment = Some(crate::catalog::CardEnchantment {
                id: EnchantmentId::Adroit,
                amount: 1,
            });
            assert!(!forge_writer_spec_is_exact(&enchanted));
            let mut alien_row = spec;
            alien_row.row = crate::content_tables::card_row(CardId::SummonForth, upgrade).unwrap();
            assert!(!forge_writer_spec_is_exact(&alien_row));
        }

        for (id, expected_cost, expected_args) in [
            (CardId::BigBang, 0, [[5, i64::MIN], [5, i64::MIN]]),
            (CardId::Bulwark, 2, [[12, 10], [15, 13]]),
            (CardId::SpoilsOfBattle, 1, [[6, 2], [9, 2]]),
        ] {
            for upgrade in 0..=1 {
                let canonical = crate::content_tables::card_row(id, upgrade).unwrap();
                assert_eq!(canonical.cost, expected_cost);
                assert!(canonical.is_skill && !canonical.is_power);
                assert!(!canonical.targeted && matches!(canonical.target_type, "Self"));
                assert!(!canonical.selects && !canonical.x_cost && !canonical.star_x);
                assert_eq!(canonical.exhausts, id == CardId::BigBang);
                assert_eq!(canonical.innate, id == CardId::BigBang && upgrade == 1);
                let ints = canonical.steps[0]
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        Arg::I(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let expected = expected_args[upgrade as usize]
                    .into_iter()
                    .filter(|value| *value != i64::MIN)
                    .collect::<Vec<_>>();
                assert_eq!(ints, expected);
                for drift in 0..7 {
                    let mut malformed = *canonical;
                    match drift {
                        0 => malformed.cost = expected_cost + 1,
                        1 => malformed.is_skill = false,
                        2 => malformed.is_power = true,
                        3 => malformed.targeted = true,
                        4 => malformed.target_type = "AnyEnemy",
                        5 => malformed.selects = true,
                        6 => malformed.x_cost = true,
                        _ => unreachable!(),
                    }
                    assert!(!forge_writer_program_is_supported(&malformed));
                }

                let mut builder = CatalogBuilder::new();
                let atom = builder
                    .intern(CardIdentity {
                        id,
                        upgrade,
                        enchantment: None,
                    })
                    .unwrap();
                let catalog = builder.build();
                let spec = *catalog.spec(atom).unwrap();
                assert!(forge_writer_spec_is_exact(&spec));
                let mut enchanted = spec;
                enchanted.identity.enchantment = Some(crate::catalog::CardEnchantment {
                    id: EnchantmentId::Adroit,
                    amount: 1,
                });
                assert!(!forge_writer_spec_is_exact(&enchanted));
            }
        }
    }

    fn blade_fixture(damage: i32) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let mut state = HotState::at_defaults();
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 30)]);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: damage,
                ..CardInstanceState::default()
            },
        );
        (state, catalog, source)
    }

    fn parry_fixture(upgrade: u8) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::Parry,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 29,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        // A live enemy keeps `damage_combat_is_ending` false: the card's
        // `PowerCmd.Apply` skips at IsEnding (#3218).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        (state, catalog, source)
    }

    fn sword_sage_fixture() -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::SwordSage,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 41,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 9;
        // A live enemy keeps `damage_combat_is_ending` false: the card's
        // `PowerCmd.Apply` skips at IsEnding (#3218).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        (state, catalog, source)
    }

    #[test]
    fn sword_sage_authenticates_live_source_piles_and_listener_lifecycle() {
        for pile in [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play] {
            let (mut state, catalog, source) = sword_sage_fixture();
            state.piles.get_mut(pile).make_mut().push(source);
            let spec = *catalog.spec(source.atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let args = catalog.args(step.args);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            crate::engine::play::with_test_active_play(source.uid, || sword_sage(&mut ctx))
                .unwrap();
            assert_eq!(state.powers.value(PowerId::SwordSage), 1, "{pile:?}");
            assert_eq!(
                state.fanouts.after_power_amount_changed_order(),
                [PowerId::SwordSage]
            );

            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            crate::engine::play::with_test_active_play(source.uid, || sword_sage(&mut ctx))
                .unwrap();
            assert_eq!(state.powers.value(PowerId::SwordSage), 2);
            assert_eq!(
                state.fanouts.after_power_amount_changed_order(),
                [PowerId::SwordSage]
            );
        }

        let (mut exhausted, catalog, source) = sword_sage_fixture();
        exhausted
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(source);
        let before = exhausted.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut exhausted,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args,
            events: &mut events,
        };
        assert!(
            crate::engine::play::with_test_active_play(source.uid, || sword_sage(&mut ctx))
                .is_err()
        );
        assert_eq!(exhausted, before);
        assert!(events.is_empty());
    }

    #[test]
    fn sword_sage_removal_preserves_peer_order_and_explicit_zero() {
        let (mut state, catalog, _) = sword_sage_fixture();
        let blade_atom = catalog
            .atom(&CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let blade = HotCard {
            uid: 51,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE | CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state.piles.get_mut(PileId::Exhaust).make_mut().push(blade);
        let mut instance = CardInstanceState::default();
        instance.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(blade.uid, instance);
        state.powers.set(PowerId::Shroud, SlotWire::Int, 1);
        state.powers.set(PowerId::SwordSage, SlotWire::Int, 2);
        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        assert!(state.fanouts.set_after_power_amount_changed_order(&[
            PowerId::Shroud,
            PowerId::SwordSage,
            PowerId::Vicious,
        ]));

        let mut events = Vec::new();
        remove_sword_sage_power(&mut state, &catalog, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::SwordSage), 0);
        assert_eq!(
            state.card_states.get(blade.uid).base_replay_count(),
            Some(0)
        );
        assert_eq!(
            state.fanouts.after_power_amount_changed_order(),
            [PowerId::Shroud, PowerId::Vicious]
        );

        let mut orphan = state.clone();
        assert!(
            orphan
                .fanouts
                .register_after_power_amount_changed(PowerId::SwordSage)
        );
        let before = orphan.clone();
        assert!(remove_sword_sage_power(&mut orphan, &catalog, &mut Vec::new()).is_err());
        assert_eq!(orphan, before);
    }

    #[test]
    fn sword_sage_public_manual_play_covers_both_costs_listener_and_blade_delta() {
        for (upgrade, cost) in [(0, 2), (1, 1)] {
            let mut builder = crate::catalog::CatalogBuilder::new();
            let sage_identity = CardIdentity {
                id: CardId::SwordSage,
                upgrade,
                enchantment: None,
            };
            let blade_identity = CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            };
            let sage_atom = builder.intern(sage_identity).unwrap();
            let blade_atom = builder.intern(blade_identity).unwrap();
            let catalog = builder.build();
            let sage = crate::hot::HotCard {
                uid: 1,
                atom: sage_atom,
                flags: 0,
            };
            let blade = crate::hot::HotCard {
                uid: 2,
                atom: blade_atom,
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
            };
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = cost;
            state.next_card_uid = 3;
            // A live enemy keeps `damage_combat_is_ending` false (#3218).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.piles.get_mut(PileId::Hand).make_mut().push(sage);
            state.piles.get_mut(PileId::Draw).make_mut().push(blade);
            let mut events = Vec::new();

            crate::engine::play::play_card(&mut state, &catalog, sage.uid, None, None, &mut events)
                .unwrap();

            assert_eq!(state.energy, 0, "upgrade {upgrade}");
            assert_eq!(state.powers.value(PowerId::SwordSage), 1);
            assert_eq!(
                state.fanouts.after_power_amount_changed_order(),
                [PowerId::SwordSage]
            );
            assert_eq!(
                state.card_states.get(blade.uid).base_replay_count(),
                Some(1)
            );
            assert_ne!(
                state.piles.get(PileId::Draw).as_slice()[0].flags
                    & crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                0
            );
            assert_eq!(state.history.owner_card_plays_finished_this_turn, 1);
            assert!(events.iter().any(|event| matches!(
                event,
                crate::engine::Event::PowerChanged {
                    power: PowerId::SwordSage,
                    amount: 1,
                    ..
                }
            )));
        }
    }

    #[test]
    fn imitation_mutable_clone_replays_sword_sage_and_updates_blade_exactly_twice() {
        let (mut state, catalog, source) = sword_sage_fixture();
        let blade_atom = catalog
            .atom(&CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let blade = crate::hot::HotCard {
            uid: 42,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.piles.get_mut(PileId::Discard).make_mut().push(blade);
        state.next_card_uid = 43;
        assert!(state.fanouts.set_imitation_learning(0, 1));

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.powers.value(PowerId::SwordSage), 2);
        assert_eq!(
            state.card_states.get(blade.uid).base_replay_count(),
            Some(2)
        );
        assert_eq!(
            state.fanouts.after_power_amount_changed_order(),
            [PowerId::SwordSage]
        );
        assert_eq!(state.fanouts.imitation_learning(0), Some(0));
        assert!(state.fanouts.imitation_clones().is_empty());
        assert_eq!(state.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn parry_public_play_covers_both_levels_and_restacks_without_a_listener() {
        for (upgrade, expected_gain) in [(0, 10), (1, 14)] {
            let (mut state, catalog, source) = parry_fixture(upgrade);
            state.powers.set(PowerId::Parry, SlotWire::Int, 3);
            let mut events = Vec::new();
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();
            assert_eq!(state.powers.value(PowerId::Parry), expected_gain + 3);
            assert!(state.fanouts.after_side_turn_start_order().is_empty());
            assert!(events.iter().any(|event| matches!(
                event,
                crate::engine::Event::PowerChanged {
                    power: PowerId::Parry,
                    amount,
                    ..
                } if *amount == expected_gain + 3
            )));
        }
    }

    #[test]
    fn parry_later_signal_boost_and_echo_overflow_roll_back_the_whole_action() {
        for (echo, prior) in [(false, i32::MAX - 20), (true, i32::MAX - 35)] {
            let (mut state, catalog, source) = parry_fixture(1);
            state.powers.set(PowerId::Parry, SlotWire::Int, prior);
            state.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            if echo {
                state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 31 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow("parry amount")),
                "Signal Boost with echo={echo} later body must refuse"
            );
            assert_eq!(
                state, before,
                "echo={echo} restored all prior bodies and prefix"
            );
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn wrought_in_war_attacks_then_forges_every_pile_and_generates_when_absent() {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder
            .intern(CardIdentity {
                id: CardId::WroughtInWar,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let blade_atom = builder
            .intern(CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom: source_atom,
            flags: 0,
        };
        let blade = HotCard {
            uid: 18,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.next_card_uid = 19;
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 30)]);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Conqueror, SlotWire::Int, 7);
        state.monsters_mut()[0].misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Conqueror]);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        for pile in [PileId::Draw, PileId::Discard, PileId::Exhaust] {
            let mut copy = blade;
            copy.uid += pile as u32;
            state.piles.get_mut(pile).make_mut().push(copy);
            state.card_states.set(
                copy.uid,
                CardInstanceState {
                    damage_growth: 10,
                    ..CardInstanceState::default()
                },
            );
        }
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 23);
        for pile in [PileId::Draw, PileId::Discard, PileId::Exhaust] {
            let card = state.piles.get(pile).as_slice()[0];
            assert_eq!(state.card_states.get(card.uid).damage_growth, 17);
        }

        let mut generated = HotState::at_defaults();
        generated.hp = 80;
        generated.max_hp = 80;
        generated.energy = 3;
        generated.next_card_uid = 40;
        generated.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 30)]);
        generated
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(source);
        crate::engine::play::play_card(
            &mut generated,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        let made = generated.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(made.uid, 40);
        assert_ne!(made.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE, 0);
        assert_eq!(generated.card_states.get(made.uid).damage_growth, 17);
        assert_eq!(generated.history.owner_generated_cards_combat, 1);
    }

    fn forge_writer_fixture(
        id: CardId,
        upgrade: u8,
    ) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::SovereignBlade,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some()
        );
        let source = HotCard {
            uid: 61,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.next_card_uid = 100;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 80);
        monster.slot = 7;
        monster.uid = 70;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        (state, catalog, source)
    }

    #[test]
    fn final_forge_modes_both_levels_have_exact_resources_and_results() {
        for (id, upgrade, forge, block, draws) in [
            (CardId::BigBang, 0, 5, 0, 1),
            (CardId::BigBang, 1, 5, 0, 1),
            (CardId::Bulwark, 0, 10, 12, 0),
            (CardId::Bulwark, 1, 13, 15, 0),
            (CardId::SpoilsOfBattle, 0, 6, 0, 2),
            (CardId::SpoilsOfBattle, 1, 9, 0, 2),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, upgrade);
            for offset in 0..draws {
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid: 200 + offset,
                    atom: source.atom,
                    flags: 0,
                });
            }
            let before_energy = state.energy;
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();

            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 10 + forge);
            assert_eq!(state.block, block);
            assert_eq!(state.piles.get(PileId::Hand).len(), draws as usize + 1);
            match id {
                CardId::BigBang => {
                    assert_eq!(state.energy, before_energy + 1);
                    assert_eq!(state.stars, 1);
                }
                CardId::Bulwark => assert_eq!(state.energy, before_energy - 2),
                CardId::SpoilsOfBattle => assert_eq!(state.energy, before_energy - 1),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn final_forge_mode_writer_order_is_explicit() {
        for (id, expected) in [
            (
                CardId::BigBang,
                &[
                    "big_bang_draw",
                    "big_bang_stars",
                    "big_bang_energy",
                    "forge",
                    "big_bang_draw",
                    "big_bang_stars",
                    "big_bang_energy",
                    "forge",
                ][..],
            ),
            (
                CardId::Bulwark,
                &["bulwark_block", "forge", "bulwark_block", "forge"][..],
            ),
            (
                CardId::SpoilsOfBattle,
                &["forge", "spoils_draw", "forge", "spoils_draw"][..],
            ),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            let spec = *catalog.spec(source.atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };
            FORGE_WRITER_TRACE.with(|trace| trace.borrow_mut().clear());
            crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
                .unwrap();
            FORGE_WRITER_TRACE.with(|trace| assert_eq!(trace.borrow().as_slice(), expected));
        }
    }

    /// #2708: Big Bang's Energy is `PlayerCmd::GainEnergy` (`BigBang/<OnPlay>d__7`
    /// 0x38c648 IL_01b4), which returns at IsEnding (0x3ee8a0 IL_0035). With
    /// the only primary dead before history.over latches, the tail's Stars
    /// and Energy both skip; while the primary lives, both land.
    #[test]
    fn big_bang_tail_energy_skips_while_combat_is_ending_before_the_over_latch() {
        let (mut live, catalog, _) = forge_writer_fixture(CardId::BigBang, 0);
        let energy = live.energy;
        apply_big_bang_tail(&mut live, &catalog, 0, &mut Vec::new()).unwrap();
        assert_eq!((live.stars, live.energy), (1, energy + 1));

        let (mut ending, catalog, _) = forge_writer_fixture(CardId::BigBang, 0);
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let energy = ending.energy;
        apply_big_bang_tail(&mut ending, &catalog, 0, &mut Vec::new()).unwrap();
        assert_eq!((ending.stars, ending.energy), (0, energy));
    }

    /// #3112: `ForgeCmd/<Forge>d__2` (0x3ed3bc IL_0025) returns at
    /// IsOverOrEnding before the live-blade query. With the only primary dead
    /// before history.over latches, Forge creates no Sovereign Blade and
    /// allocates no uid; while the primary lives, it generates one blade.
    #[test]
    fn forge_skips_while_combat_is_ending_before_the_over_latch() {
        let (mut ending, catalog, _) = forge_writer_fixture(CardId::BigBang, 0);
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(crate::engine::damage::damage_combat_is_ending(&ending));
        let before = ending.clone();
        let mut events = Vec::new();
        forge_exact(&mut ending, &catalog, 5, &mut events).unwrap();
        assert_eq!(ending, before);
        assert!(events.is_empty());

        let (mut live, catalog, _) = forge_writer_fixture(CardId::BigBang, 0);
        let uid = live.next_card_uid;
        let hand = live.piles.get(PileId::Hand).as_slice().len();
        forge_exact(&mut live, &catalog, 5, &mut Vec::new()).unwrap();
        assert_eq!(live.next_card_uid, uid + 1);
        assert_eq!(live.piles.get(PileId::Hand).as_slice().len(), hand + 1);
    }

    /// #3218: Refine Blade's `EnergyNextTurnPower`, Furnace, Hammer Time,
    /// Parry and Sword Sage are each one `PowerCmd.Apply<T>` (MethodDef
    /// `0x0600560d`), whose ``<Apply>d__1`1`` (`0x3ef988` IL_0020-0034)
    /// returns at IsEnding. With the only primary dead before history.over
    /// latches, and again once over, every writer is a total no-op (Furnace
    /// and Hammer Time formerly refused on over and wrote while ending); a
    /// live primary applies each power.
    #[test]
    fn regent_power_applies_skip_while_combat_is_ending_before_the_over_latch() {
        type Writer = fn(&mut StepCtx<'_>) -> Result<(), EngineRefusal>;
        type Live = fn(&HotState) -> bool;
        fn in_play(mut state: HotState, source: HotCard) -> HotState {
            for pile in [PileId::Hand, PileId::Play] {
                state
                    .piles
                    .get_mut(pile)
                    .make_mut()
                    .retain(|card| card.uid != source.uid);
            }
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            state
        }
        fn hammer_fixture() -> (HotState, crate::catalog::Catalog, HotCard) {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::HammerTime,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let source = HotCard {
                uid: 12,
                atom,
                flags: 0,
            };
            (hammer_party(), builder.build(), source)
        }
        let furnace = || {
            let (state, catalog, source, _) = furnace_fixture(0);
            (state, catalog, source)
        };
        type Fixture = fn() -> (HotState, crate::catalog::Catalog, HotCard);
        let cases: [(&str, Fixture, Writer, Live); 5] = [
            (
                "Refine Blade EnergyNextTurn",
                || forge_writer_fixture(CardId::RefineBlade, 0),
                forge_family_exact,
                |s| s.powers.value(PowerId::EnergyNextTurn) == 1,
            ),
            ("Furnace", furnace, furnace_foundation_exact, |s| {
                s.powers.value(PowerId::Furnace) == 5
            }),
            ("Hammer Time", hammer_fixture, hammer_time_exact, |s| {
                s.fanouts.hammer_time()
            }),
            (
                "Parry",
                || parry_fixture(0),
                parry,
                |s| s.powers.value(PowerId::Parry) == 10,
            ),
            ("Sword Sage", sword_sage_fixture, sword_sage, |s| {
                s.powers.value(PowerId::SwordSage) == 1
            }),
        ];
        for (name, fixture, writer, live) in cases {
            let run = |state: &mut HotState| {
                let (_, catalog, source) = fixture();
                let spec = *catalog.spec(source.atom).unwrap();
                let step = catalog.steps(&spec)[0];
                let mut events = vec![crate::engine::Event::TurnEnded { turn: 23 }];
                let before_events = events.clone();
                let result = crate::engine::play::with_test_active_play(source.uid, || {
                    writer(&mut StepCtx {
                        state,
                        catalog: &catalog,
                        spec: &spec,
                        source_uid: source.uid,
                        target: None,
                        selection: None,
                        x_value: 0,
                        args: catalog.args(step.args),
                        events: &mut events,
                    })
                });
                (result, events == before_events)
            };
            let fresh = || {
                let (state, _, source) = fixture();
                in_play(state, source)
            };

            let mut ending = fresh();
            ending.monsters_mut()[0].hp = 0;
            assert!(!ending.history.over, "{name}");
            assert!(
                crate::engine::damage::damage_combat_is_ending(&ending),
                "{name}"
            );
            let before = ending.clone();
            assert_eq!(run(&mut ending), (Ok(()), true), "{name} ending");
            assert_eq!(ending, before, "{name} is a no-op while ending");

            let mut over = fresh();
            over.history.over = true;
            let before = over.clone();
            assert_eq!(run(&mut over), (Ok(()), true), "{name} over");
            assert_eq!(over, before, "{name} is a no-op once over");

            let mut applied = fresh();
            assert_eq!(run(&mut applied).0, Ok(()), "{name} live");
            assert!(live(&applied), "{name} applies while combat is live");
        }
    }

    #[test]
    fn big_bang_draws_a_blade_before_forge_and_terminal_star_suppresses_suffix() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::BigBang, 0);
        forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = state.piles.get_mut(PileId::Hand).make_mut().pop().unwrap();
        state.piles.get_mut(PileId::Draw).make_mut().push(blade);
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.piles.get(PileId::Hand).as_slice().contains(&blade));
        assert_eq!(state.card_states.get(blade.uid).damage_growth, 15);
        assert_eq!(state.stars, 1);
        assert_eq!(state.energy, 4);

        let (mut draw_terminal, catalog, source) = forge_writer_fixture(CardId::BigBang, 0);
        draw_terminal
            .powers
            .set(PowerId::Cacophony, SlotWire::Int, 100);
        assert!(
            draw_terminal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        draw_terminal.fanouts.set_cacophony_left(1);
        draw_terminal.monsters_mut()[0].hp = 1;
        draw_terminal
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 210,
                atom: source.atom,
                flags: 0,
            });
        crate::engine::play::play_card(
            &mut draw_terminal,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(draw_terminal.history.over);
        assert_eq!((draw_terminal.stars, draw_terminal.energy), (0, 3));
        assert!(!PileId::ALL.into_iter().any(|pile| {
            draw_terminal.piles.get(pile).as_slice().iter().any(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
        }));

        let (mut terminal, catalog, source) = forge_writer_fixture(CardId::BigBang, 0);
        forge_exact(&mut terminal, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = terminal.piles.get(PileId::Hand).as_slice()[1];
        terminal.powers.set(PowerId::BlackHole, SlotWire::Int, 100);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut terminal);
        terminal.monsters_mut()[0].hp = 1;
        terminal
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 211,
                atom: source.atom,
                flags: 0,
            });
        crate::engine::play::play_card(
            &mut terminal,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(terminal.history.over);
        assert_eq!(terminal.stars, 1, "GainStars completed before Black Hole");
        assert_eq!(terminal.energy, 3, "the ending gate suppresses GainEnergy");
        assert_eq!(
            terminal.card_states.get(blade.uid).damage_growth,
            10,
            "the ending gate suppresses Forge"
        );
        assert!(
            terminal
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| card.uid == 211)
        );
    }

    #[test]
    fn command_draw_gates_preserve_each_final_forge_mode_prefix() {
        let (mut no_draw, catalog, source) = forge_writer_fixture(CardId::BigBang, 0);
        no_draw.powers.set(PowerId::NoDraw, SlotWire::Int, 1);
        no_draw
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 220,
                atom: source.atom,
                flags: 0,
            });
        crate::engine::play::play_card(
            &mut no_draw,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(no_draw.piles.get(PileId::Draw).len(), 1);
        assert_eq!(no_draw.stars, 1);
        assert_eq!(no_draw.energy, 4);

        let (mut full, catalog, source) = forge_writer_fixture(CardId::SpoilsOfBattle, 0);
        for uid in 230..239 {
            full.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: source.atom,
                flags: 0,
            });
        }
        for uid in 240..242 {
            full.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom: source.atom,
                flags: 0,
            });
        }
        crate::engine::play::play_card(
            &mut full,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(full.piles.get(PileId::Hand).len(), 10);
        assert_eq!(full.piles.get(PileId::Draw).len(), 2);
        assert!(full.piles.get(PileId::Hand).as_slice().iter().any(|card| {
            catalog
                .spec(card.atom)
                .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
        }));
    }

    #[test]
    fn final_forge_mode_late_failures_restore_the_whole_action() {
        for (id, configure, expected) in [
            (
                CardId::BigBang,
                0_u8,
                EngineRefusal::CounterOverflow("energy"),
            ),
            (
                CardId::Bulwark,
                1_u8,
                EngineRefusal::CounterOverflow("sovereign blade damage"),
            ),
            (
                CardId::SpoilsOfBattle,
                2_u8,
                EngineRefusal::CounterOverflow("cards_drawn_combat"),
            ),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            match configure {
                0 => {
                    state.energy = i16::MAX;
                    state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                        uid: 250,
                        atom: source.atom,
                        flags: 0,
                    });
                }
                1 => {
                    forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
                    let blade = state.piles.get(PileId::Hand).as_slice()[1];
                    state.card_states.set(
                        blade.uid,
                        CardInstanceState {
                            damage_growth: i32::MAX - 5,
                            ..CardInstanceState::default()
                        },
                    );
                }
                2 => {
                    state.cards_drawn_combat = i32::MAX;
                    state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                        uid: 251,
                        atom: source.atom,
                        flags: 0,
                    });
                }
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 37 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(expected),
                "{id:?}"
            );
            assert_eq!(state, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");
        }
    }

    #[test]
    fn final_forge_mode_second_replay_failure_restores_the_first_body_and_prefix() {
        for (id, forge) in [
            (CardId::BigBang, 5),
            (CardId::Bulwark, 10),
            (CardId::SpoilsOfBattle, 6),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
            let blade = state.piles.get(PileId::Hand).as_slice()[1];
            state.card_states.set(
                blade.uid,
                CardInstanceState {
                    damage_growth: i32::MAX - forge - 1,
                    ..CardInstanceState::default()
                },
            );
            state.powers.set(PowerId::Burst, SlotWire::Int, 1);
            for uid in 252..256 {
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid,
                    atom: source.atom,
                    flags: 0,
                });
            }
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 38 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow("sovereign blade damage")),
                "{id:?}"
            );
            assert_eq!(state, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");
        }
    }

    #[test]
    fn new_draw_readers_invalidate_old_shortcuts_and_refuse_before_action_prefix() {
        for id in [CardId::BigBang, CardId::SpoilsOfBattle] {
            let (mut listener, catalog, source) = forge_writer_fixture(id, 0);
            listener.powers.set(PowerId::Accuracy, SlotWire::Int, 1);
            assert!(
                listener
                    .fanouts
                    .set_after_card_drawn_order(&[PowerId::Accuracy])
            );
            listener
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(HotCard {
                    uid: 280,
                    atom: source.atom,
                    flags: 0,
                });
            let before = listener.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 40 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut listener,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::PowerHookNotModeled {
                    power: PowerId::Accuracy,
                    event: HookEvent::AfterCardDrawn,
                }),
                "{id:?} AfterCardDrawn"
            );
            assert_eq!(listener, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");

            let (mut shuffle, catalog, source) = forge_writer_fixture(id, 0);
            shuffle.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
            shuffle.piles.get_mut(PileId::Discard).make_mut().extend([
                HotCard {
                    uid: 281,
                    atom: source.atom,
                    flags: 0,
                },
                HotCard {
                    uid: 282,
                    atom: source.atom,
                    flags: 0,
                },
            ]);
            let before = shuffle.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 41 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut shuffle,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("stratagem selection")),
                "{id:?} AfterShuffle"
            );
            assert_eq!(shuffle, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");
        }
    }

    #[test]
    fn final_forge_modes_burst_and_echo_replay_but_signal_boost_does_not() {
        for id in [CardId::BigBang, CardId::Bulwark, CardId::SpoilsOfBattle] {
            let per_body = match id {
                CardId::BigBang => 5,
                CardId::Bulwark => 10,
                CardId::SpoilsOfBattle => 6,
                _ => unreachable!(),
            };
            for replay in [PowerId::Burst, PowerId::EchoForm] {
                let (mut repeated, catalog, source) = forge_writer_fixture(id, 0);
                repeated.powers.set(replay, SlotWire::Int, 1);
                for uid in 260..264 {
                    repeated
                        .piles
                        .get_mut(PileId::Draw)
                        .make_mut()
                        .push(HotCard {
                            uid,
                            atom: source.atom,
                            flags: 0,
                        });
                }
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(
                    &mut repeated,
                );
                crate::engine::play::play_card(
                    &mut repeated,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut Vec::new(),
                )
                .unwrap();
                let blade = PileId::ALL
                    .into_iter()
                    .flat_map(|pile| repeated.piles.get(pile).as_slice())
                    .find(|card| {
                        catalog
                            .spec(card.atom)
                            .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                    })
                    .unwrap();
                assert_eq!(
                    repeated.card_states.get(blade.uid).damage_growth,
                    10 + 2 * per_body
                );
            }

            let (mut signal, catalog, source) = forge_writer_fixture(id, 0);
            signal.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            for uid in 270..272 {
                signal.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid,
                    atom: source.atom,
                    flags: 0,
                });
            }
            crate::engine::play::play_card(
                &mut signal,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| signal.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(
                signal.card_states.get(blade.uid).damage_growth,
                10 + per_body
            );
        }
    }

    #[test]
    fn bulwark_replay_clamps_storage_but_publishes_each_full_gain_and_forge_body() {
        for replay in [PowerId::Burst, PowerId::EchoForm] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::Bulwark, 0);
            state.block = 999_999_997;
            state.powers.set(replay, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let mut events = Vec::new();
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();

            assert_eq!(state.block, 999_999_999);
            assert_eq!(state.history.card_block_gains, 2);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        crate::engine::Event::PlayerBlockGained {
                            amount: 12,
                            block: 999_999_999,
                        }
                    ))
                    .count(),
                2
            );
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 30);
        }
    }

    #[test]
    fn bulwark_capped_block_rolls_back_on_later_forge_or_listener_refusal() {
        for listener in [false, true] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::Bulwark, 0);
            state.block = 999_999_999;
            let expected = if listener {
                state.powers.set(PowerId::Accuracy, SlotWire::Int, 1);
                assert!(
                    state
                        .fanouts
                        .set_after_block_gained_order(&[PowerId::Accuracy])
                );
                EngineRefusal::PowerHookNotModeled {
                    power: PowerId::Accuracy,
                    event: HookEvent::AfterBlockGained,
                }
            } else {
                forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
                let blade = state.piles.get(PileId::Hand).as_slice()[1];
                state.card_states.set(
                    blade.uid,
                    CardInstanceState {
                        damage_growth: i32::MAX - 5,
                        ..CardInstanceState::default()
                    },
                );
                EngineRefusal::CounterOverflow("sovereign blade damage")
            };
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 43 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(expected),
                "listener={listener}"
            );
            assert_eq!(state, before, "listener={listener}");
            assert_eq!(events, before_events, "listener={listener}");
        }
    }

    #[test]
    fn final_forge_modes_collected_autoplay_use_generic_three_pile_provenance_without_spend() {
        for id in [CardId::BigBang, CardId::Bulwark, CardId::SpoilsOfBattle] {
            for pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
                let (mut state, catalog, source) = forge_writer_fixture(id, 0);
                state.piles.get_mut(PileId::Hand).make_mut().clear();
                state.piles.get_mut(pile).make_mut().push(source);
                crate::engine::play::autoplay_collected_cards(
                    &mut state,
                    &catalog,
                    &[source],
                    &mut Vec::new(),
                )
                .unwrap_or_else(|refusal| panic!("{id:?} from {pile:?}: {refusal}"));
                match id {
                    CardId::BigBang => assert_eq!((state.energy, state.stars), (4, 1)),
                    CardId::Bulwark => assert_eq!((state.energy, state.block), (3, 12)),
                    CardId::SpoilsOfBattle => assert_eq!(state.energy, 3),
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn final_forge_modes_preserve_safe_inexact_singletons_and_promote_divergent_siblings() {
        for id in [CardId::BigBang, CardId::Bulwark, CardId::SpoilsOfBattle] {
            let (mut singleton, catalog, source) = forge_writer_fixture(id, 0);
            forge_exact(&mut singleton, &catalog, 0, &mut Vec::new()).unwrap();
            singleton.exact_piles = false;
            crate::engine::play::play_card(
                &mut singleton,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(!singleton.exact_piles, "safe singleton {id:?}");

            let (mut divergent, catalog, source) = forge_writer_fixture(id, 0);
            forge_exact(&mut divergent, &catalog, 0, &mut Vec::new()).unwrap();
            let first = divergent.piles.get(PileId::Hand).as_slice()[1];
            let second = HotCard {
                uid: 292,
                atom: first.atom,
                flags: first.flags,
            };
            divergent
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(second);
            divergent.card_states.set(
                second.uid,
                CardInstanceState {
                    damage_growth: 20,
                    ..CardInstanceState::default()
                },
            );
            divergent.exact_piles = false;
            crate::engine::play::play_card(
                &mut divergent,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(divergent.exact_piles, "divergent siblings {id:?}");
        }
    }

    #[test]
    fn final_forge_modes_refuse_enchanted_physical_sources_before_the_play_prefix() {
        for id in [CardId::BigBang, CardId::Bulwark, CardId::SpoilsOfBattle] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: Some(crate::catalog::CardEnchantment {
                        id: EnchantmentId::Momentum,
                        amount: 1,
                    }),
                })
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let source = HotCard {
                uid: 291,
                atom,
                flags: 0,
            };
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.energy = 5;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 80));
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 42 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "exact Forge writer rehearsal source"
                )),
                "{id:?}"
            );
            assert_eq!(state, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");
        }
    }

    #[test]
    fn final_forge_mode_terminal_bodies_are_total_noops_after_source_authentication() {
        for id in [CardId::BigBang, CardId::Bulwark, CardId::SpoilsOfBattle] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            state.history.over = true;
            let before = state.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 39 }];
            let before_events = events.clone();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args: catalog.args(step.args),
                events: &mut events,
            };
            crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
                .unwrap();
            assert_eq!(state, before, "{id:?}");
            assert_eq!(events, before_events, "{id:?}");
        }
    }

    #[test]
    fn final_forge_modes_compose_with_furnace_refine_seeking_conqueror_and_parry() {
        for (id, expected_damage, writer_block) in [
            (CardId::BigBang, 15, 0),
            (CardId::Bulwark, 20, 12),
            (CardId::SpoilsOfBattle, 16, 0),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            state.energy = 5;
            state.monsters_mut()[0].hp = 100;
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Conqueror, SlotWire::Int, 2);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(MiseryToken::Conqueror);
            let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
            second.uid = 72;
            second.slot = 8;
            state.monsters_mut().push(second);
            state.powers.set(PowerId::Furnace, SlotWire::Int, 5);
            assert!(
                state.fanouts.set_after_side_turn_start_order(&[
                    crate::hot::AfterSideTurnStartToken::Furnace
                ])
            );
            state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 1);
            state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
            state.powers.set(PowerId::Parry, SlotWire::Int, 8);

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .copied()
                .unwrap();
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                blade.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                (state.monsters[0].hp, state.monsters[1].hp),
                (100 - 2 * expected_damage, 100 - expected_damage)
            );
            assert_eq!(state.block, writer_block + 8);
            assert_eq!(state.powers.value(PowerId::Furnace), 5);
            assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
        }
    }

    #[test]
    fn seeking_edge_both_levels_apply_unique_before_each_replayed_forge() {
        for (upgrade, forge) in [(0, 7), (1, 11)] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::SeekingEdge, upgrade);
            state.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
            let mut events = Vec::new();
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();

            assert_eq!(state.powers.value(PowerId::SeekingEdge), 1);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        crate::engine::Event::PowerChanged {
                            power: PowerId::SeekingEdge,
                            ..
                        }
                    ))
                    .count(),
                1,
                "Unique replay must not reapply the power"
            );
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(
                state.card_states.get(blade.uid).damage_growth,
                10 + forge * 2
            );
        }
    }

    #[test]
    fn conqueror_both_levels_forge_then_apply_and_acquire_misery_once() {
        for (upgrade, forge) in [(0, 3), (1, 5)] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::Conqueror, upgrade);
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 10 + forge);
            assert_eq!(state.monsters[0].powers.value(PowerId::Conqueror), 1);
            assert_eq!(
                state.monsters[0].misery_debuff_order.as_slice(),
                &[MiseryToken::Conqueror]
            );
        }
    }

    #[test]
    fn refine_blade_both_levels_forge_then_apply_energy_next_turn() {
        for (upgrade, forge) in [(0, 8), (1, 12)] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, upgrade);
            let mut events = Vec::new();
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();

            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 10 + forge);
            assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        crate::engine::Event::PowerChanged {
                            subject: crate::engine::Subject::Player,
                            power: PowerId::EnergyNextTurn,
                            amount: 1,
                        }
                    ))
                    .count(),
                1
            );
            assert!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == source.uid)
            );
        }
    }

    #[test]
    fn summon_forth_both_levels_move_one_ordered_plural_batch_then_forge() {
        for (upgrade, forge) in [(0, 8), (1, 11)] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::SummonForth, upgrade);
            let blade_atom = catalog
                .atom(&CardIdentity {
                    id: CardId::SovereignBlade,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let mut expected = Vec::new();
            for (uid, pile) in [
                (71, PileId::Hand),
                (72, PileId::Discard),
                (73, PileId::Exhaust),
                (74, PileId::Play),
            ] {
                let card = HotCard {
                    uid,
                    atom: blade_atom,
                    flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
                };
                state.piles.get_mut(pile).make_mut().push(card);
                state.card_states.set(
                    uid,
                    CardInstanceState {
                        damage_growth: 10,
                        ..CardInstanceState::default()
                    },
                );
                expected.push(uid);
            }
            let existing_draw = HotCard {
                uid: 75,
                atom: source.atom,
                flags: 0,
            };
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .push(existing_draw);
            let mut events = Vec::new();

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
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
                std::iter::once(existing_draw.uid)
                    .chain(expected.iter().copied())
                    .collect::<Vec<_>>()
            );
            for uid in &expected {
                let moved = state
                    .piles
                    .get(PileId::Draw)
                    .as_slice()
                    .iter()
                    .find(|card| card.uid == *uid)
                    .unwrap();
                assert_eq!(moved.flags, CARD_FLAG_SOVEREIGN_BLADE_STATE);
                assert_eq!(state.card_states.get(*uid).damage_growth, 10 + forge);
            }
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        crate::engine::Event::CardResolved {
                            uid,
                            pile: PileId::Draw,
                        } => Some(*uid),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                expected
            );
            let first_move = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        crate::engine::Event::CardResolved {
                            uid: 71,
                            pile: PileId::Draw
                        }
                    )
                })
                .unwrap();
            assert_eq!(
                &events[first_move..first_move + 4],
                &[
                    crate::engine::Event::CardResolved {
                        uid: 71,
                        pile: PileId::Draw,
                    },
                    crate::engine::Event::CardResolved {
                        uid: 72,
                        pile: PileId::Draw,
                    },
                    crate::engine::Event::CardResolved {
                        uid: 73,
                        pile: PileId::Draw,
                    },
                    crate::engine::Event::CardResolved {
                        uid: 74,
                        pile: PileId::Draw,
                    },
                ],
                "one plural command publishes its ordered batch contiguously"
            );
        }
    }

    #[test]
    fn the_smith_both_levels_spend_four_stars_then_forge() {
        for (upgrade, forge) in [(0, 30), (1, 40)] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::TheSmith, upgrade);
            state.stars = 4;
            let mut events = Vec::new();

            assert_eq!(
                crate::engine::legal_actions(&state, &catalog)
                    .into_iter()
                    .filter_map(|action| match action {
                        crate::engine::Action::Play { uid, target, .. } if uid == source.uid => {
                            Some(target)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                vec![None],
                "The Smith+{upgrade} has exactly one untargeted play action"
            );

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .unwrap();

            assert_eq!((state.energy, state.stars), (2, 0));
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 10 + forge);
            assert_eq!(state.history.owner_generated_cards_combat, 1);
            assert!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == source.uid)
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, crate::engine::Event::CardPlayed { uid, .. } if *uid == source.uid))
                    .count(),
                1
            );
        }

        let (mut short, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        short.stars = 3;
        let before = short.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 90 }];
        let before_events = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut short,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::NotEnoughStars { cost: 4, stars: 3 })
        );
        assert_eq!(short, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_smith_burst_and_echo_replay_bodies_but_signal_boost_does_not() {
        for replay in [PowerId::Burst, PowerId::EchoForm] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
            state.stars = 4;
            state.powers.set(replay, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 70);
            assert_eq!((state.energy, state.stars), (2, 0));
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
        }

        let (mut signal, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        signal.stars = 4;
        signal.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
        crate::engine::play::play_card(
            &mut signal,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        let blade = PileId::ALL
            .into_iter()
            .flat_map(|pile| signal.piles.get(pile).as_slice())
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .unwrap();
        assert_eq!(signal.card_states.get(blade.uid).damage_growth, 40);
        assert_eq!(signal.powers.value(PowerId::SignalBoost), 1);
    }

    #[test]
    fn the_smith_direct_autoplay_authenticates_every_source_pile_without_spending() {
        for pile in PileId::ALL {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
            state.stars = 4;
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(pile).make_mut().push(source);
            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[source],
                &mut Vec::new(),
            )
            .unwrap_or_else(|refusal| panic!("AutoPlay from {pile:?}: {refusal}"));
            assert_eq!((state.energy, state.stars), (3, 4));
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|candidate| state.piles.get(candidate).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 40);
        }
    }

    #[test]
    fn collected_autoplay_all_pile_exception_is_exactly_summon_and_the_smith() {
        for pile in [PileId::Exhaust, PileId::Play] {
            for id in [
                CardId::SummonForth,
                CardId::TheSmith,
                CardId::SeekingEdge,
                CardId::Conqueror,
                CardId::RefineBlade,
                CardId::BigBang,
                CardId::Bulwark,
                CardId::SpoilsOfBattle,
            ] {
                let (mut state, catalog, source) = forge_writer_fixture(id, 0);
                state.piles.get_mut(PileId::Hand).make_mut().clear();
                state.piles.get_mut(pile).make_mut().push(source);
                let before = state.clone();
                let mut events = vec![crate::engine::Event::TurnBegan { turn: 92 }];
                let before_events = events.clone();
                let result = crate::engine::play::autoplay_collected_cards(
                    &mut state,
                    &catalog,
                    &[source],
                    &mut events,
                );

                if matches!(id, CardId::SummonForth | CardId::TheSmith) {
                    result.unwrap_or_else(|refusal| {
                        panic!("{id:?} direct AutoPlay from {pile:?}: {refusal}")
                    });
                    assert_ne!(state, before);
                    assert!(PileId::ALL.into_iter().any(|candidate| {
                        state.piles.get(candidate).as_slice().iter().any(|card| {
                            catalog.spec(card.atom).is_some_and(|spec| {
                                matches!(spec.identity.id, CardId::SovereignBlade)
                            })
                        })
                    }));
                } else {
                    assert_eq!(result, Err(EngineRefusal::ContinuationNotModeled));
                    assert_eq!(state, before, "{id:?} from {pile:?}");
                    assert_eq!(events, before_events, "{id:?} from {pile:?}");
                }
            }
        }
    }

    #[test]
    fn the_smith_later_replay_overflow_restores_stars_prefix_and_events() {
        for replay in [PowerId::Burst, PowerId::EchoForm] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
            state.stars = 4;
            forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
            let blade = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .find(|card| card.uid != source.uid)
                .copied()
                .unwrap();
            state.card_states.set(
                blade.uid,
                CardInstanceState {
                    damage_growth: i32::MAX - 30,
                    ..CardInstanceState::default()
                },
            );
            state.powers.set(replay, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 91 }];
            let before_events = events.clone();

            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow("sovereign blade damage"))
            );
            assert_eq!(state, before, "{replay:?}");
            assert_eq!(events, before_events, "{replay:?}");
        }
    }

    #[test]
    fn the_smith_terminal_body_is_inert_but_live_target_and_duplicate_source_refuse() {
        let (mut terminal, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        terminal.piles.get_mut(PileId::Hand).make_mut().clear();
        terminal.piles.get_mut(PileId::Play).make_mut().push(source);
        terminal.history.over = true;
        let before = terminal.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 17 }];
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut terminal,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();
        assert_eq!(terminal, before);
        assert_eq!(events, before_events);

        let (mut targeted, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        targeted.piles.get_mut(PileId::Hand).make_mut().clear();
        targeted.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = targeted.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut targeted,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        assert_eq!(
            crate::engine::play::with_test_active_play(source.uid, || {
                forge_family_exact(&mut ctx)
            }),
            Err(EngineRefusal::MalformedArgs("forge_family_exact target"))
        );
        assert_eq!(targeted, before);
        assert!(events.is_empty());

        let (mut duplicate, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        duplicate.stars = 4;
        duplicate
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(source);
        let before = duplicate.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 18 }];
        let before_events = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut duplicate,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("forge_family_exact source"))
        );
        assert_eq!(duplicate, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_smith_enchanted_source_refuses_before_resource_or_pile_prefix() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::TheSmith,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment {
                    id: EnchantmentId::Momentum,
                    amount: 1,
                }),
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 61,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.stars = 4;
        state.next_card_uid = 100;
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 80)]);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 19 }];
        let before_events = events.clone();

        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "exact Forge writer rehearsal source"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn the_smith_exhaust_only_generates_then_grows_both_and_exactness_is_minimal() {
        let (mut exhausted, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        exhausted.stars = 4;
        let blade_atom = catalog
            .atom(&CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let old = HotCard {
            uid: 71,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        exhausted
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(old);
        exhausted.card_states.set(
            old.uid,
            CardInstanceState {
                damage_growth: 20,
                ..CardInstanceState::default()
            },
        );
        crate::engine::play::play_card(
            &mut exhausted,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        let generated = exhausted
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| card.uid != source.uid)
            .copied()
            .unwrap();
        assert_eq!(exhausted.card_states.get(generated.uid).damage_growth, 40);
        assert_eq!(exhausted.card_states.get(old.uid).damage_growth, 50);
        assert_eq!(exhausted.history.owner_generated_cards_combat, 1);

        let (mut singleton, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        singleton.stars = 4;
        forge_exact(&mut singleton, &catalog, 0, &mut Vec::new()).unwrap();
        singleton.exact_piles = false;
        crate::engine::play::play_card(
            &mut singleton,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!singleton.exact_piles, "one Blade has no order ambiguity");

        let (mut divergent, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        divergent.stars = 4;
        forge_exact(&mut divergent, &catalog, 0, &mut Vec::new()).unwrap();
        let first = divergent
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| card.uid != source.uid)
            .copied()
            .unwrap();
        let second = HotCard { uid: 72, ..first };
        divergent
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(second);
        divergent.card_states.set(
            second.uid,
            CardInstanceState {
                damage_growth: 20,
                ..CardInstanceState::default()
            },
        );
        divergent.next_card_uid = 100;
        divergent.exact_piles = false;
        crate::engine::play::play_card(
            &mut divergent,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(
            divergent.exact_piles,
            "distinguishable equal-identity Hand siblings require true order"
        );
    }

    #[test]
    fn summon_forth_replay_rereads_piles_and_exhaust_only_never_generates() {
        for replay in [PowerId::Burst, PowerId::EchoForm] {
            let (mut generated, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
            generated.powers.set(replay, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut generated);
            crate::engine::play::play_card(
                &mut generated,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            let blade = generated
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .find(|card| {
                    matches!(
                        catalog.spec(card.atom).unwrap().identity.id,
                        CardId::SovereignBlade
                    )
                })
                .unwrap();
            assert_eq!(generated.history.owner_generated_cards_combat, 1);
            assert_eq!(generated.card_states.get(blade.uid).damage_growth, 26);
        }

        let (mut exhausted, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        forge_exact(&mut exhausted, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = exhausted
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .pop()
            .unwrap();
        exhausted
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(blade);
        let generated_before = exhausted.history.owner_generated_cards_combat;
        crate::engine::play::play_card(
            &mut exhausted,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            exhausted.history.owner_generated_cards_combat,
            generated_before
        );
        assert!(exhausted.piles.get(PileId::Exhaust).as_slice().is_empty());
        assert_eq!(
            exhausted.piles.get(PileId::Draw).as_slice()[0].uid,
            blade.uid
        );
        assert_eq!(exhausted.card_states.get(blade.uid).damage_growth, 18);
    }

    #[test]
    fn summon_forth_refuses_an_active_outer_blade_and_rolls_back_the_action() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = state.piles.get(PileId::Hand).as_slice()[1];
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 41 }];
        let before_events = events.clone();

        let refusal = crate::engine::play::with_test_active_play(blade.uid, || {
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
        });
        assert_eq!(
            refusal,
            Err(EngineRefusal::MalformedArgs(
                "physical move frozen identity"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn summon_forth_terminal_body_is_an_observational_noop_for_represented_cards() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        state.history.over = true;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 99 }];
        let before_events = events.clone();
        let spec = catalog.spec(source.atom).unwrap();
        let step = catalog.steps(spec)[0];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        forge_family_exact(&mut ctx).unwrap();
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn summon_forth_exact_pile_gate_is_minimal_and_physical_source_owned() {
        let (base, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        let blade_atom = catalog
            .atom(&CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let blade = |uid| HotCard {
            uid,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let assert_safe_inexact_move = |mut state: HotState, label: &str| {
            state.next_card_uid = 100;
            state.exact_piles = false;
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap_or_else(|refusal| panic!("{label}: {refusal}"));
            assert!(!state.exact_piles, "{label}");
        };

        let mut one = base.clone();
        one.piles.get_mut(PileId::Hand).make_mut().push(blade(71));
        assert!(!crate::engine::admission::summon_forth_needs_exact_piles(
            &one, &catalog
        ));
        assert_safe_inexact_move(one.clone(), "one Blade");

        let mut identical = one.clone();
        identical
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(blade(72));
        assert!(!crate::engine::admission::summon_forth_needs_exact_piles(
            &identical, &catalog
        ));
        assert_safe_inexact_move(identical.clone(), "identical siblings");
        identical.card_states.set(
            72,
            CardInstanceState {
                damage_growth: 1,
                ..CardInstanceState::default()
            },
        );
        assert!(crate::engine::admission::summon_forth_needs_exact_piles(
            &identical, &catalog
        ));

        let mut split = one.clone();
        split
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(blade(72));
        split.card_states.set(
            72,
            CardInstanceState {
                damage_growth: 1,
                ..CardInstanceState::default()
            },
        );
        assert!(!crate::engine::admission::summon_forth_needs_exact_piles(
            &split, &catalog
        ));

        let mut exhaust_play = base.clone();
        exhaust_play
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .extend([blade(71), blade(72)]);
        exhaust_play.card_states.set(
            72,
            CardInstanceState {
                damage_growth: 1,
                ..CardInstanceState::default()
            },
        );
        assert!(!crate::engine::admission::summon_forth_needs_exact_piles(
            &exhaust_play,
            &catalog
        ));

        let mut catalog_only = identical;
        catalog_only
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .retain(|card| card.uid != source.uid);
        assert!(!crate::engine::admission::summon_forth_needs_exact_piles(
            &catalog_only,
            &catalog
        ));
    }

    #[test]
    fn summon_forth_manual_and_direct_autoplay_reach_every_source_pile() {
        let (mut manual, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        crate::engine::play::play_card(
            &mut manual,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(manual.energy, 2);

        for pile in PileId::ALL {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(pile).make_mut().push(source);
            let energy = state.energy;
            crate::engine::play::autoplay_collected_cards(
                &mut state,
                &catalog,
                &[source],
                &mut Vec::new(),
            )
            .unwrap_or_else(|refusal| panic!("AutoPlay from {pile:?}: {refusal}"));
            assert_eq!(state.energy, energy, "AutoPlay from {pile:?}");
            let blade = PileId::ALL
                .into_iter()
                .flat_map(|candidate| state.piles.get(candidate).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 18);
        }

        let (mut full, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        full.piles.get_mut(PileId::Hand).make_mut().clear();
        full.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((20..30).map(|uid| HotCard {
                uid,
                atom: source.atom,
                flags: 0,
            }));
        full.piles.get_mut(PileId::Draw).make_mut().push(source);
        full.next_card_uid = 30;
        crate::engine::play::autoplay_collected_cards(
            &mut full,
            &catalog,
            &[source],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(full.piles.get(PileId::Hand).len(), 10);
        let generated = full
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .unwrap();
        assert_eq!(full.history.owner_generated_cards_combat, 1);
        assert_eq!(full.card_states.get(generated.uid).damage_growth, 18);
    }

    #[test]
    fn summon_forth_later_replay_failure_restores_move_and_shared_play_prefix() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::SummonForth, 0);
        forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| card.uid != source.uid)
            .copied()
            .unwrap();
        state.card_states.set(
            blade.uid,
            CardInstanceState {
                damage_growth: i32::MAX - 12,
                ..CardInstanceState::default()
            },
        );
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 42 }];
        let before_events = events.clone();

        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("sovereign blade damage"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn refine_blade_replay_routes_once_and_repeats_complete_bodies() {
        for replay in [PowerId::Burst, PowerId::EchoForm] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
            state.powers.set(replay, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();

            let blade = PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
                .unwrap();
            assert_eq!(state.card_states.get(blade.uid).damage_growth, 26);
            assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 2);
            assert_eq!(
                PileId::ALL
                    .into_iter()
                    .flat_map(|pile| state.piles.get(pile).as_slice())
                    .filter(|card| card.uid == source.uid)
                    .count(),
                1
            );
            assert!(
                state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == source.uid)
            );
        }

        let (mut signal, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        signal.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
        crate::engine::play::play_card(
            &mut signal,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(signal.powers.value(PowerId::EnergyNextTurn), 1);
        let blade = PileId::ALL
            .into_iter()
            .flat_map(|pile| signal.piles.get(pile).as_slice())
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .unwrap();
        assert_eq!(signal.card_states.get(blade.uid).damage_growth, 18);
    }

    #[test]
    fn refine_blade_collected_autoplay_is_no_target_and_preserves_manual_cost() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        let energy_before = state.energy;

        crate::engine::play::autoplay_collected_cards(
            &mut state,
            &catalog,
            &[source],
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            state.energy, energy_before,
            "AutoPlay does not spend printed cost"
        );
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
        assert!(
            PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .any(|card| {
                    catalog
                        .spec(card.atom)
                        .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                })
        );
    }

    #[test]
    fn refine_blade_accepts_every_complete_inert_type_one_listener_permutation() {
        for order in [
            [PowerId::Vicious, PowerId::Shroud, PowerId::SleightOfFlesh],
            [PowerId::Vicious, PowerId::SleightOfFlesh, PowerId::Shroud],
            [PowerId::Shroud, PowerId::Vicious, PowerId::SleightOfFlesh],
            [PowerId::Shroud, PowerId::SleightOfFlesh, PowerId::Vicious],
            [PowerId::SleightOfFlesh, PowerId::Vicious, PowerId::Shroud],
            [PowerId::SleightOfFlesh, PowerId::Shroud, PowerId::Vicious],
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
            for power in order {
                state.powers.set(power, SlotWire::Int, 1);
            }
            assert!(state.fanouts.set_after_power_amount_changed_order(&order));

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
            assert_eq!(state.fanouts.after_power_amount_changed_order(), order);
        }
    }

    #[test]
    fn refine_blade_malformed_type_one_listener_sets_refuse_atomically() {
        for shape in 0..5 {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
            match shape {
                0 => {
                    state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
                    assert!(state.fanouts.set_after_power_amount_changed_order(&[]));
                }
                1 => {
                    assert!(
                        state
                            .fanouts
                            .set_after_power_amount_changed_order(&[PowerId::Vicious])
                    );
                }
                2 => {
                    state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
                    assert!(state.fanouts.set_after_power_amount_changed_order(&[
                        PowerId::Vicious,
                        PowerId::Vicious,
                    ]));
                }
                3 => state.powers.set(PowerId::Vicious, SlotWire::Int, -1),
                4 => {
                    state.powers.set(PowerId::Vicious, SlotWire::Bool, 1);
                    assert!(
                        state
                            .fanouts
                            .set_after_power_amount_changed_order(&[PowerId::Vicious])
                    );
                }
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 31 }];
            let before_events = events.clone();

            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                )),
                "shape {shape}"
            );
            assert_eq!(state, before, "shape {shape}");
            assert_eq!(events, before_events, "shape {shape}");
        }
    }

    #[test]
    fn refine_blade_generation_handles_full_hand_and_exhaust_only_blade() {
        let (mut full, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        full.piles.get_mut(PileId::Hand).make_mut().clear();
        full.piles.get_mut(PileId::Play).make_mut().push(source);
        for uid in 200..210 {
            full.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: source.atom,
                flags: 0,
            });
        }
        full.next_card_uid = 300;
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut full,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();
        let generated = full
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .copied()
            .unwrap();
        assert_eq!(full.card_states.get(generated.uid).damage_growth, 18);
        assert_eq!(full.powers.value(PowerId::EnergyNextTurn), 1);

        let (mut exhausted, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        forge_exact(&mut exhausted, &catalog, 0, &mut Vec::new()).unwrap();
        let old = exhausted
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .pop()
            .unwrap();
        exhausted
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(old);
        exhausted.piles.get_mut(PileId::Hand).make_mut().clear();
        exhausted
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(source);
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut exhausted,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();
        let blades = [PileId::Hand, PileId::Exhaust]
            .into_iter()
            .flat_map(|pile| exhausted.piles.get(pile).as_slice())
            .filter(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .collect::<Vec<_>>();
        assert_eq!(blades.len(), 2);
        assert!(
            blades
                .iter()
                .all(|card| exhausted.card_states.get(card.uid).damage_growth == 18)
        );
    }

    #[test]
    fn refine_blade_deferred_stack_grants_full_amount_once_then_removes() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 3);
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 4);

        let transition =
            crate::engine::apply_action(&state, &catalog, &crate::engine::Action::EndTurn).unwrap();

        assert_eq!(transition.state.energy, 7, "base reset plus the full stack");
        assert_eq!(transition.state.powers.value(PowerId::EnergyNextTurn), 0);
        assert!(transition.events.iter().any(|event| matches!(
            event,
            crate::engine::Event::PowerChanged {
                subject: crate::engine::Subject::Player,
                power: PowerId::EnergyNextTurn,
                amount: 0,
            }
        )));
    }

    #[test]
    fn refine_blade_terminal_entry_skips_forge_and_energy_suffix() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        state.history.over = true;
        let before = state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 19 }];
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };

        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();

        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn refine_blade_late_power_failures_restore_the_whole_action() {
        for (label, configure, expected) in [
            (
                "first body overflow",
                0_u8,
                EngineRefusal::CounterOverflow("energy next turn"),
            ),
            (
                "later Burst body overflow",
                1_u8,
                EngineRefusal::CounterOverflow("energy next turn"),
            ),
            (
                "malformed listener order",
                2_u8,
                EngineRefusal::MalformedArgs("after-power-amount-changed listener order"),
            ),
            (
                "malformed EnergyNextTurn wire",
                3_u8,
                EngineRefusal::MalformedArgs("EnergyNextTurn power state"),
            ),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
            match configure {
                0 => state
                    .powers
                    .set(PowerId::EnergyNextTurn, SlotWire::Int, i32::MAX),
                1 => {
                    state
                        .powers
                        .set(PowerId::EnergyNextTurn, SlotWire::Int, i32::MAX - 1);
                    state.powers.set(PowerId::Burst, SlotWire::Int, 1);
                }
                2 => state.powers.set(PowerId::Vicious, SlotWire::Int, 1),
                3 => state.powers.set(PowerId::EnergyNextTurn, SlotWire::Bool, 1),
                _ => unreachable!(),
            }
            if configure == 1 {
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            }
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 23 }];
            let before_events = events.clone();

            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(expected),
                "{label}"
            );
            assert_eq!(state, before, "{label}");
            assert_eq!(events, before_events, "{label}");
        }

        let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        forge_exact(&mut state, &catalog, 0, &mut Vec::new()).unwrap();
        let blade = state.piles.get(PileId::Hand).as_slice()[1];
        state.card_states.set(
            blade.uid,
            CardInstanceState {
                damage_growth: i32::MAX - 4,
                ..CardInstanceState::default()
            },
        );
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 29 }];
        let before_events = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("sovereign blade damage"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn seeking_and_conqueror_writer_operation_order_is_explicit() {
        for (id, target, expected) in [
            (
                CardId::SeekingEdge,
                None,
                [
                    "seeking_power_changed",
                    "forge",
                    "seeking_power_changed",
                    "forge",
                ],
            ),
            (
                CardId::Conqueror,
                Some(0),
                [
                    "forge",
                    "conqueror_power_changed",
                    "forge",
                    "conqueror_power_changed",
                ],
            ),
        ] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(PileId::Play).make_mut().push(source);
            let spec = *catalog.spec(source.atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let args = catalog.args(step.args);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target,
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            FORGE_WRITER_TRACE.with(|trace| trace.borrow_mut().clear());
            if target.is_some() {
                let identity = (ctx.state.monsters[0].slot, ctx.state.monsters[0].uid);
                crate::engine::play::with_test_active_play_target(source.uid, identity, || {
                    forge_family_exact(&mut ctx)
                })
                .unwrap();
            } else {
                crate::engine::play::with_test_active_play(source.uid, || {
                    forge_family_exact(&mut ctx)
                })
                .unwrap();
            }
            FORGE_WRITER_TRACE.with(|trace| assert_eq!(trace.borrow().as_slice(), expected));
            let power = if id == CardId::SeekingEdge {
                PowerId::SeekingEdge
            } else {
                PowerId::Conqueror
            };
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, crate::engine::Event::PowerChanged {
                        power: changed,
                        ..
                    } if *changed == power))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn refine_blade_writer_order_is_forge_before_power_publication() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::RefineBlade, 0);
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        FORGE_WRITER_TRACE.with(|trace| trace.borrow_mut().clear());

        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();

        FORGE_WRITER_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                [
                    "forge",
                    "refine_energy_next_turn",
                    "forge",
                    "refine_energy_next_turn",
                ]
            )
        });
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
    }

    #[test]
    fn the_smith_writer_trace_contains_only_the_single_forge_command() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::TheSmith, 0);
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let spec = *catalog.spec(source.atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: catalog.args(step.args),
            events: &mut events,
        };
        FORGE_WRITER_TRACE.with(|trace| trace.borrow_mut().clear());

        crate::engine::play::with_test_active_play(source.uid, || forge_family_exact(&mut ctx))
            .unwrap();

        FORGE_WRITER_TRACE.with(|trace| assert_eq!(trace.borrow().as_slice(), ["forge", "forge"]));
        assert!(events.iter().all(|event| !matches!(
            event,
            crate::engine::Event::PowerChanged { .. }
                | crate::engine::Event::MonsterDamaged { .. }
                | crate::engine::Event::PlayerBlockGained { .. }
        )));
    }

    #[test]
    fn conqueror_post_forge_apply_failure_restores_the_whole_action() {
        let (mut state, catalog, source) = forge_writer_fixture(CardId::Conqueror, 0);
        state.fanouts.set_unsettling_lamp_available(true);
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Conqueror, SlotWire::Int, i32::MAX - 2);
        state.monsters_mut()[0].misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Conqueror]);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 77 }];
        let before_events = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("monster debuff"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn conqueror_artifact_lamp_stack_and_after_amount_listener_are_exact() {
        let (mut blocked, catalog, source) = forge_writer_fixture(CardId::Conqueror, 0);
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        blocked.fanouts.set_unsettling_lamp_available(true);
        crate::engine::play::play_card(
            &mut blocked,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Conqueror), 0);
        assert!(blocked.fanouts.unsettling_lamp_available());
        assert!(
            blocked.monsters[0]
                .misery_debuff_order
                .as_slice()
                .is_empty()
        );

        let (mut replay, catalog, source) = forge_writer_fixture(CardId::Conqueror, 0);
        replay.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut replay);
        replay.fanouts.set_unsettling_lamp_available(true);
        crate::engine::play::play_card(
            &mut replay,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(replay.monsters[0].powers.value(PowerId::Conqueror), 3);
        assert!(!replay.fanouts.unsettling_lamp_available());
        assert_eq!(
            replay.monsters[0].misery_debuff_order.as_slice(),
            &[MiseryToken::Conqueror]
        );

        let (mut listener, catalog, source) = forge_writer_fixture(CardId::Conqueror, 0);
        listener
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 5);
        assert!(
            listener
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        crate::engine::play::play_card(
            &mut listener,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(listener.monsters[0].hp, 75);
        assert_eq!(listener.monsters[0].powers.value(PowerId::Conqueror), 1);
    }

    #[test]
    fn seeking_sovereign_is_one_roster_order_aoe_without_target_rng() {
        let (mut state, catalog, source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut first = HotMonster::new(MonsterKind::Toadpole, 15);
        first.slot = 3;
        first.uid = 30;
        first.powers.set(PowerId::Conqueror, SlotWire::Int, 4);
        first.misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Conqueror]);
        let mut dead = HotMonster::new(MonsterKind::Toadpole, 0);
        dead.slot = 4;
        dead.uid = 40;
        let mut last = HotMonster::new(MonsterKind::Toadpole, 25);
        last.slot = 5;
        last.uid = 50;
        state.monsters = std::sync::Arc::new(vec![first, dead, last]);
        let targets_before = state.rng.get(crate::hot::RngStream::Targets);
        let mut events = Vec::new();
        crate::engine::play::play_card(&mut state, &catalog, source.uid, None, None, &mut events)
            .unwrap();

        assert_eq!(
            state.rng.get(crate::hot::RngStream::Targets),
            targets_before
        );
        assert_eq!(
            (
                state.monsters[0].hp,
                state.monsters[1].hp,
                state.monsters[2].hp
            ),
            (-11, 0, 12)
        );
        assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 1);
        assert_eq!(state.monsters[2].owner_powered_damage_results_this_turn, 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Conqueror), 0);
        assert!(state.monsters[0].misery_debuff_order.as_slice().is_empty());
        assert_eq!(state.powers.value(PowerId::Vigor), 0);
        let damaged = events
            .iter()
            .filter_map(|event| match event {
                crate::engine::Event::MonsterDamaged { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            damaged,
            [30, 50],
            "one command preserves living roster order"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    crate::engine::Event::PowerChanged {
                        subject: crate::engine::Subject::Player,
                        power: PowerId::Vigor,
                        amount: 0,
                    }
                ))
                .count(),
            1,
            "one command snapshots Vigor for both targets and removes it once"
        );
    }

    #[test]
    fn seeking_target_shape_is_shared_by_manual_direct_and_draw_top_autoplay() {
        let (state, catalog, source) = blade_fixture(10);
        for draw_top in [false, true] {
            let mut case = state.clone();
            case.piles.get_mut(PileId::Play).make_mut().clear();
            let pile = if draw_top {
                PileId::Draw
            } else {
                PileId::Discard
            };
            case.piles.get_mut(pile).make_mut().push(source);
            case.hp = 80;
            case.max_hp = 80;
            case.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
            let mut second = HotMonster::new(MonsterKind::Toadpole, 30);
            second.slot = 2;
            second.uid = 2;
            case.monsters_mut().push(second);
            let targets_before = case.rng.get(crate::hot::RngStream::Targets);
            if draw_top {
                crate::engine::play::autoplay_draw_top(&mut case, &catalog, 1, &mut Vec::new())
                    .unwrap();
            } else {
                crate::engine::play::autoplay_collected_cards(
                    &mut case,
                    &catalog,
                    &[source],
                    &mut Vec::new(),
                )
                .unwrap();
            }
            assert_eq!(case.rng.get(crate::hot::RngStream::Targets), targets_before);
            assert_eq!((case.monsters[0].hp, case.monsters[1].hp), (20, 20));
        }

        let (mut drift, catalog, source) = blade_fixture(10);
        drift.piles.get_mut(PileId::Play).make_mut().clear();
        drift.piles.get_mut(PileId::Hand).make_mut().push(source);
        drift.hp = 80;
        drift.energy = 3;
        drift.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        let before = drift.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut drift,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::TargetMismatch { required: false })
        );
        assert_eq!(drift, before);

        for (wire, value) in [(SlotWire::Bool, 1), (SlotWire::Int, 2)] {
            let (mut malformed, catalog, source) = forge_writer_fixture(CardId::SeekingEdge, 0);
            malformed.powers.set(PowerId::SeekingEdge, wire, value);
            let before = malformed.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut malformed,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut Vec::new(),
                ),
                Err(EngineRefusal::MalformedArgs("Seeking Edge power state"))
            );
            assert_eq!(malformed, before);
        }
    }

    #[test]
    fn sovereign_conqueror_amount_is_duration_and_multiplier_composes() {
        for conqueror in [0, 1, 9] {
            let (mut state, catalog, source) = blade_fixture(10);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.hp = 80;
            state.energy = 3;
            state.monsters_mut()[0].hp = 200;
            state.monsters_mut()[0].max_hp = 200;
            state.powers.set(PowerId::DoubleDamage, SlotWire::Int, 1);
            state.powers.set(PowerId::PlayerWeak, SlotWire::Int, 1);
            state.powers.set(PowerId::Tracking, SlotWire::Int, 50);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Vuln, SlotWire::Int, 1);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Weak, SlotWire::Int, 1);
            if conqueror > 0 {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Conqueror, SlotWire::Int, conqueror);
            }
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                state.monsters[0].hp,
                if conqueror == 0 { 167 } else { 133 },
                "positive duration applies exactly one x2"
            );
        }
    }

    #[test]
    fn furnace_created_blade_uses_seeking_conqueror_then_parry() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::Furnace,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.next_card_uid = 71;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 50);
        monster.uid = 9;
        monster.slot = 9;
        monster.powers.set(PowerId::Conqueror, SlotWire::Int, 3);
        monster.misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Conqueror]);
        state.monsters_mut().push(monster);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 50);
        second.uid = 10;
        second.slot = 10;
        state.monsters_mut().push(second);
        forge_exact(&mut state, &catalog, 5, &mut Vec::new()).unwrap();
        let blade = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(state.card_states.get(blade.uid).damage_growth, 15);
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        state.powers.set(PowerId::Parry, SlotWire::Int, 8);
        let mut events = Vec::new();
        crate::engine::play::play_card(&mut state, &catalog, blade.uid, None, None, &mut events)
            .unwrap();
        assert_eq!((state.monsters[0].hp, state.monsters[1].hp), (20, 35));
        assert_eq!(state.block, 8);
        let damages = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                matches!(event, crate::engine::Event::MonsterDamaged { .. }).then_some(index)
            })
            .collect::<Vec<_>>();
        let block = events
            .iter()
            .position(|event| matches!(event, crate::engine::Event::PlayerBlockGained { .. }))
            .unwrap();
        assert_eq!(damages.len(), 2);
        assert!(damages.into_iter().all(|damage| damage < block));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::engine::Event::PlayerBlockGained { .. }))
                .count(),
            1,
            "Parry applies once after the complete multi-target command"
        );
    }

    #[test]
    fn refine_blade_composes_with_furnace_seeking_conqueror_and_parry() {
        let mut builder = CatalogBuilder::new();
        let refine_atom = builder
            .intern(CardIdentity {
                id: CardId::RefineBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: CardId::Furnace,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 81,
            atom: refine_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.energy = 3;
        state.next_card_uid = 82;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let mut first = HotMonster::new(MonsterKind::Toadpole, 100);
        first.uid = 91;
        first.slot = 4;
        first.powers.set(PowerId::Conqueror, SlotWire::Int, 3);
        first.misery_debuff_order.push(MiseryToken::Conqueror);
        state.monsters_mut().push(first);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
        second.uid = 92;
        second.slot = 5;
        state.monsters_mut().push(second);
        state.powers.set(PowerId::Furnace, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        state.powers.set(PowerId::Parry, SlotWire::Int, 8);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        let blade = PileId::ALL
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .find(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
            })
            .copied()
            .unwrap();
        assert_eq!(state.card_states.get(blade.uid).damage_growth, 18);
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            blade.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((state.monsters[0].hp, state.monsters[1].hp), (64, 82));
        assert_eq!(state.block, 8);
        assert_eq!(state.powers.value(PowerId::Furnace), 5);
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
    }

    #[test]
    fn the_smith_composes_after_summon_with_furnace_refine_seeking_conqueror_and_parry() {
        let mut builder = CatalogBuilder::new();
        let smith_atom = builder
            .intern(CardIdentity {
                id: CardId::TheSmith,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let summon_atom = builder
            .intern(CardIdentity {
                id: CardId::SummonForth,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern(CardIdentity {
                id: CardId::Furnace,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let blade_atom = catalog
            .atom(&CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let smith = HotCard {
            uid: 81,
            atom: smith_atom,
            flags: 0,
        };
        let summon = HotCard {
            uid: 82,
            atom: summon_atom,
            flags: 0,
        };
        let blade = HotCard {
            uid: 83,
            atom: blade_atom,
            flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.energy = 3;
        state.stars = 4;
        state.next_card_uid = 84;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([summon, smith]);
        state.piles.get_mut(PileId::Discard).make_mut().push(blade);
        state.card_states.set(
            blade.uid,
            CardInstanceState {
                damage_growth: 10,
                ..CardInstanceState::default()
            },
        );
        let mut first = HotMonster::new(MonsterKind::Toadpole, 100);
        first.uid = 91;
        first.slot = 4;
        first.powers.set(PowerId::Conqueror, SlotWire::Int, 3);
        first.misery_debuff_order.push(MiseryToken::Conqueror);
        state.monsters_mut().push(first);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
        second.uid = 92;
        second.slot = 5;
        state.monsters_mut().push(second);
        state.powers.set(PowerId::Furnace, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 1);
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        state.powers.set(PowerId::Parry, SlotWire::Int, 8);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            summon.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.card_states.get(blade.uid).damage_growth, 18);
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            smith.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.card_states.get(blade.uid).damage_growth, 48);
        crate::engine::play::autoplay_collected_cards(
            &mut state,
            &catalog,
            &[blade],
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!((state.monsters[0].hp, state.monsters[1].hp), (4, 52));
        assert_eq!(state.block, 8);
        assert_eq!((state.energy, state.stars), (1, 0));
        assert_eq!(state.powers.value(PowerId::Furnace), 5);
        assert_eq!(state.powers.value(PowerId::EnergyNextTurn), 1);
    }

    #[test]
    fn sovereign_later_aoe_target_refusal_restores_the_complete_action() {
        let (mut state, catalog, source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.energy = 3;
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        let mut later = HotMonster::new(MonsterKind::Toadpole, 50);
        later.uid = 2;
        later.slot = 2;
        later.powers.set(PowerId::Conqueror, SlotWire::Int, 1);
        later.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        later.misery_debuff_order =
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Conqueror]);
        state.monsters_mut().push(later);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnBegan { turn: 12 }];
        let before_events = events.clone();
        assert!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            )
            .is_err()
        );
        assert_eq!(
            state, before,
            "target zero damage and shared prefix roll back"
        );
        assert_eq!(events, before_events);
    }

    #[test]
    fn sovereign_refuses_malformed_conqueror_before_any_aoe_target_leaks() {
        for (wire, value) in [(SlotWire::Bool, 1), (SlotWire::Int, -1)] {
            let (mut state, catalog, source) = blade_fixture(10);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.hp = 80;
            state.energy = 3;
            state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
            let mut later = HotMonster::new(MonsterKind::Toadpole, 50);
            later.uid = 2;
            later.slot = 2;
            later.powers.set(PowerId::Conqueror, wire, value);
            state.monsters_mut().push(later);
            let before = state.clone();
            let mut events = vec![crate::engine::Event::TurnBegan { turn: 12 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    None,
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs("Conqueror power state"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn forge_refuses_a_mismatched_physical_source_before_mutation() {
        let mut builder = CatalogBuilder::new();
        let wrought_atom = builder
            .intern(CardIdentity {
                id: CardId::WroughtInWar,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let beat_atom = builder
            .intern(CardIdentity {
                id: CardId::BeatIntoShape,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom: beat_atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 30)]);
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = state.clone();
        let spec = *catalog.spec(wrought_atom).unwrap();
        let args = [
            CompiledArg::Word(StepWord::WroughtInWar),
            CompiledArg::I(7),
            CompiledArg::I(7),
        ];
        let mut events = Vec::new();
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &args,
            events: &mut events,
        };
        assert!(matches!(
            forge_family_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("forge_family_exact source"))
        ));
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn new_writers_authenticate_all_piles_but_existing_writers_remain_play_only() {
        for id in [
            CardId::SeekingEdge,
            CardId::Conqueror,
            CardId::RefineBlade,
            CardId::SummonForth,
            CardId::TheSmith,
            CardId::BigBang,
            CardId::Bulwark,
            CardId::SpoilsOfBattle,
        ] {
            for pile in PileId::ALL {
                let (mut state, catalog, source) = forge_writer_fixture(id, 0);
                state.piles.get_mut(PileId::Hand).make_mut().clear();
                state.piles.get_mut(pile).make_mut().push(source);
                let spec = *catalog.spec(source.atom).unwrap();
                let step = catalog.steps(&spec)[0];
                let args = catalog.args(step.args);
                let target = (id == CardId::Conqueror).then_some(0);
                let mut events = Vec::new();
                let mut ctx = StepCtx {
                    state: &mut state,
                    catalog: &catalog,
                    spec: &spec,
                    source_uid: source.uid,
                    target,
                    selection: None,
                    x_value: 0,
                    args,
                    events: &mut events,
                };
                if id == CardId::Conqueror {
                    let identity = (ctx.state.monsters[0].slot, ctx.state.monsters[0].uid);
                    crate::engine::play::with_test_active_play_target(source.uid, identity, || {
                        forge_family_exact(&mut ctx)
                    })
                    .unwrap();
                    assert_eq!(state.monsters[0].powers.value(PowerId::Conqueror), 1);
                } else {
                    crate::engine::play::with_test_active_play(source.uid, || {
                        forge_family_exact(&mut ctx)
                    })
                    .unwrap();
                    if matches!(
                        id,
                        CardId::SummonForth
                            | CardId::TheSmith
                            | CardId::BigBang
                            | CardId::Bulwark
                            | CardId::SpoilsOfBattle
                    ) {
                        assert!(PileId::ALL.into_iter().any(|pile| {
                            state.piles.get(pile).as_slice().iter().any(|card| {
                                catalog.spec(card.atom).is_some_and(|spec| {
                                    matches!(spec.identity.id, CardId::SovereignBlade)
                                })
                            })
                        }));
                        if id == CardId::TheSmith {
                            let blade = PileId::ALL
                                .into_iter()
                                .flat_map(|pile| state.piles.get(pile).as_slice())
                                .find(|card| {
                                    catalog.spec(card.atom).is_some_and(|spec| {
                                        matches!(spec.identity.id, CardId::SovereignBlade)
                                    })
                                })
                                .unwrap();
                            assert_eq!(state.card_states.get(blade.uid).damage_growth, 40);
                        }
                        match id {
                            CardId::BigBang => assert_eq!((state.energy, state.stars), (4, 1)),
                            CardId::Bulwark => assert_eq!(state.block, 12),
                            _ => {}
                        }
                    } else {
                        let power = if id == CardId::SeekingEdge {
                            PowerId::SeekingEdge
                        } else {
                            PowerId::EnergyNextTurn
                        };
                        assert_eq!(state.powers.value(power), 1);
                    }
                }
            }
        }

        for id in [CardId::BeatIntoShape, CardId::WroughtInWar] {
            let (mut state, catalog, source) = forge_writer_fixture(id, 0);
            state.piles.get_mut(PileId::Hand).make_mut().clear();
            state.piles.get_mut(PileId::Discard).make_mut().push(source);
            let before = state.clone();
            let spec = *catalog.spec(source.atom).unwrap();
            let step = catalog.steps(&spec)[0];
            let args = catalog.args(step.args);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args,
                events: &mut events,
            };
            assert_eq!(
                forge_family_exact(&mut ctx),
                Err(EngineRefusal::MalformedArgs("forge_family_exact source"))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn sovereign_blade_authenticates_each_pile_and_rejects_duplicate_uid() {
        for pile in PileId::ALL {
            let (mut state, catalog, source) = blade_fixture(13);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state.piles.get_mut(pile).make_mut().push(source);
            let spec = *catalog.spec(source.atom).unwrap();
            let target_identity = (state.monsters[0].slot, state.monsters[0].uid);
            let mut events = Vec::new();
            let mut ctx = StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: source.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &[],
                events: &mut events,
            };
            crate::engine::play::with_test_active_play_target(source.uid, target_identity, || {
                sovereign_blade_exact(&mut ctx)
            })
            .unwrap();
            assert_eq!(state.monsters[0].hp, 17, "{pile:?}");
        }

        let (mut state, catalog, source) = blade_fixture(13);
        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        let before = state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let target_identity = (state.monsters[0].slot, state.monsters[0].uid);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };
        assert_eq!(
            crate::engine::play::with_test_active_play_target(source.uid, target_identity, || {
                sovereign_blade_exact(&mut ctx)
            },),
            Err(EngineRefusal::MalformedArgs(
                "sovereign_blade_exact physical source"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn sovereign_blade_reads_live_slot_six_damage_and_attacks_once() {
        let (mut state, catalog, source) = blade_fixture(13);
        let spec = *catalog.spec(source.atom).unwrap();
        let target_identity = (state.monsters[0].slot, state.monsters[0].uid);
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };
        crate::engine::play::with_test_active_play_target(source.uid, target_identity, || {
            sovereign_blade_exact(&mut ctx)
        })
        .unwrap();
        assert_eq!(state.monsters[0].hp, 17);
        assert_eq!(state.card_states.get(source.uid).damage_growth, 13);
    }

    #[test]
    fn sovereign_blade_public_play_preserves_payload_and_routes_after_attack() {
        let (mut state, catalog, source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        let mut events = Vec::new();
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.energy, 1);
        assert_eq!(state.monsters[0].hp, 20);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert!(state.piles.get(PileId::Play).is_empty());
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
        assert_eq!(state.card_states.get(source.uid).damage_growth, 10);
        assert!(matches!(
            events.last(),
            Some(crate::engine::Event::CardResolved {
                uid: 17,
                pile: PileId::Discard
            })
        ));
    }

    /// Two-body Sovereign Blade series whose first body kills its target
    /// while another monster lives, so the combat is not ending.
    fn lethal_blade_replay(
        relics: &[crate::ids::RelicId],
        parry: i32,
    ) -> (HotState, Vec<crate::engine::Event>, HotCard) {
        let (mut state, _, mut source) = blade_fixture(10);
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(relics).unwrap();
        let catalog = builder.build();
        assert_eq!(atom, source.atom);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.monsters_mut()[0].hp = 10;
        state.monsters_mut()[0].max_hp = 10;
        let mut survivor = HotMonster::new(MonsterKind::Toadpole, 100);
        survivor.max_hp = 100;
        survivor.slot = 1;
        survivor.uid = 1;
        state.monsters_mut().push(survivor);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        if parry > 0 {
            state.powers.set(PowerId::Parry, SlotWire::Int, parry);
        }
        let mut instance = state.card_states.get(source.uid);
        instance.set_base_replay_count(Some(1)).unwrap();
        state.card_states.set(source.uid, instance);
        let mut events = Vec::new();
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();
        (state, events, source)
    }

    /// #3101: `OnPlayWrapper` (`0x31b8d0`) exits between bodies only on
    /// `IsOverOrEnding` (IL_0428-IL_0432), and Sovereign Blade's own body
    /// (`0x3bd568`) has no dead-target return. The second body therefore
    /// starts and finishes against the dead target, its attack no-ops (the
    /// survivor is untouched), and the card still routes exactly once.
    #[test]
    fn sovereign_replay_runs_every_body_after_a_lethal_body_and_routes_once() {
        let (state, events, source) = lethal_blade_replay(&[], 0);

        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[1].hp, 100);
        assert!(!state.history.over);
        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::engine::Event::CardResolved { .. }))
                .count(),
            1
        );
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
    }

    /// #3101: the dead-target body still runs BeforeCardPlayed, so Pen Nib
    /// (`relics::before_card_played_hand`) advances once per body.
    #[test]
    fn sovereign_replay_dead_target_body_still_advances_pen_nib() {
        let (state, _, _) = lethal_blade_replay(&[crate::ids::RelicId::RelicPenNib], 0);
        assert_eq!(state.fanouts.pen_nib(), 2);
        assert_eq!(state.monsters[1].hp, 100);
    }

    /// #3101: `<OnPlay>d__25` reads `GetOwnerParryAmount` after the awaited
    /// attack (IL_0220-IL_027b) with no target test, so the dead-target body
    /// still gains the Parry block.
    #[test]
    fn sovereign_replay_dead_target_body_still_gains_parry_block() {
        let (with_parry, _, _) = lethal_blade_replay(&[], 5);
        let (without, _, _) = lethal_blade_replay(&[], 0);
        assert_eq!(without.block, 0);
        assert_eq!(with_parry.block, 10);
    }

    #[test]
    fn sovereign_replay_allows_256_bodies_and_refuses_257_before_mutation() {
        for (replay, succeeds) in [(255, true), (256, false)] {
            let (mut state, catalog, mut source) = blade_fixture(0);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.energy = 3;
            state.hp = 80;
            state.max_hp = 80;
            let mut instance = state.card_states.get(source.uid);
            instance.set_base_replay_count(Some(replay)).unwrap();
            state.card_states.set(source.uid, instance);
            let action = crate::engine::Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: crate::engine::SelectionRef::NONE,
            };
            assert_eq!(
                crate::engine::legal_actions(&state, &catalog).contains(&action),
                succeeds,
                "replay={replay} action offering"
            );
            let before = state.clone();
            let mut events = Vec::new();
            let result = crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            );
            if succeeds {
                result.unwrap();
                assert_eq!(state.history.card_plays_finished_combat, 256);
                assert_eq!(state.energy, 1);
                assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
            } else {
                assert_eq!(result, Err(EngineRefusal::CounterOverflow("play count")));
                assert_eq!(state, before);
                assert!(events.is_empty());
            }
        }
    }

    #[test]
    fn sovereign_replay_snapshots_zero_one_two_punch_and_echo_compositions() {
        for (replay, one_two_punch, echo, expected) in [
            (0, false, false, 1),
            (1, true, true, 4),
            (254, true, false, 256),
        ] {
            let (mut state, catalog, mut source) = blade_fixture(0);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.hp = 80;
            state.max_hp = 80;
            state.energy = 3;
            let mut instance = state.card_states.get(source.uid);
            instance.set_base_replay_count(Some(replay)).unwrap();
            state.card_states.set(source.uid, instance);
            if one_two_punch {
                state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
            }
            if echo {
                state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
            }
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

            assert!(crate::engine::legal_actions(&state, &catalog).contains(
                &crate::engine::Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: crate::engine::SelectionRef::NONE,
                }
            ));

            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.history.card_plays_finished_combat, expected);
            assert_eq!(state.powers.value(PowerId::OneTwoPunch), 0);
            assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
        }

        for (replay, one_two_punch, echo) in
            [(255, true, false), (255, false, true), (254, true, true)]
        {
            let (mut state, catalog, mut source) = blade_fixture(0);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.hp = 80;
            state.max_hp = 80;
            state.energy = 3;
            if one_two_punch {
                state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
            }
            if echo {
                state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
            }
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let mut instance = state.card_states.get(source.uid);
            instance.set_base_replay_count(Some(replay)).unwrap();
            state.card_states.set(source.uid, instance);
            let before = state.clone();
            let mut events = Vec::new();
            assert!(!crate::engine::legal_actions(&state, &catalog).iter().any(
                |action| matches!(action, crate::engine::Action::Play { uid, .. } if *uid == source.uid)
            ));
            assert_eq!(
                crate::engine::play::play_card(
                    &mut state,
                    &catalog,
                    source.uid,
                    Some(0),
                    None,
                    &mut events,
                ),
                Err(EngineRefusal::CounterOverflow("play count"))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn juggling_mutable_clone_preserves_base_replay_and_original_runs_all_bodies() {
        let (mut state, catalog, mut source) = blade_fixture(1);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.next_card_uid = 90;
        state.powers.set(PowerId::Juggling, SlotWire::Int, 1);
        state.powers.set(PowerId::JugglingAttacks, SlotWire::Int, 2);
        let mut instance = state.card_states.get(source.uid);
        instance.set_base_replay_count(Some(1)).unwrap();
        state.card_states.set(source.uid, instance);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.history.card_plays_finished_combat, 2);
        assert_eq!(state.monsters[0].hp, 28);
        let clone = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(clone.uid, 90);
        assert_eq!(clone.flags, source.flags);
        assert_eq!(
            state.card_states.get(clone.uid).base_replay_count(),
            Some(1)
        );
        assert_eq!(state.powers.value(PowerId::JugglingAttacks), 4);
    }

    #[test]
    fn sovereign_terminal_replay_suppresses_remaining_bodies_and_stays_in_play() {
        let (mut state, catalog, mut source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.monsters_mut()[0].hp = 10;
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        let mut instance = state.card_states.get(source.uid);
        instance.set_base_replay_count(Some(1)).unwrap();
        state.card_states.set(source.uid, instance);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.history.over);
        assert_eq!(state.history.card_plays_finished_combat, 1);
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
        assert!(state.piles.get(PileId::Discard).is_empty());
    }

    #[test]
    fn sovereign_body_256_failure_rolls_back_the_whole_action() {
        let (mut state, catalog, mut source) = blade_fixture(0);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.energy = 3;
        state.hp = 80;
        state.max_hp = 80;
        state.history.owner_card_plays_finished_this_turn = i16::MAX - 255;
        let mut instance = state.card_states.get(source.uid);
        instance.set_base_replay_count(Some(255)).unwrap();
        state.card_states.set(source.uid, instance);
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 9 }];
        let before_events = events.clone();

        assert_eq!(
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                Some(0),
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "owner_card_plays_finished_this_turn"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn sovereign_blade_attacks_then_gains_live_powered_parry_block() {
        let (mut state, catalog, source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.powers.set(PowerId::Parry, SlotWire::Int, 11);
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 2);
        let mut events = Vec::new();
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 20);
        assert_eq!(state.block, 13);
        let damage = events
            .iter()
            .position(|event| matches!(event, crate::engine::Event::MonsterDamaged { .. }))
            .unwrap();
        let block = events
            .iter()
            .position(|event| matches!(event, crate::engine::Event::PlayerBlockGained { .. }))
            .unwrap();
        assert!(
            damage < block,
            "the awaited attack must finish before Block"
        );
    }

    #[test]
    fn sovereign_blade_terminal_attack_reaches_the_live_parry_noop() {
        let (mut state, catalog, source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.monsters_mut()[0].hp = 5;
        state.powers.set(PowerId::Parry, SlotWire::Int, 17);
        let mut events = Vec::new();
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.powers.value(PowerId::Parry), 17);
        assert_eq!(state.block, 0);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, crate::engine::Event::PlayerBlockGained { .. }))
        );
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
    }

    #[test]
    fn sovereign_blade_accepts_disjoint_cost_and_retain_physical_state() {
        let (mut state, catalog, mut source) = blade_fixture(10);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        source.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let payload = CardInstanceState {
            damage_growth: 10,
            local_cost_modifiers: LocalCostModifiers::from_rows(vec![LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]),
            local_retain: true,
            transient_retain: true,
            ..CardInstanceState::default()
        };
        state.card_states.set(source.uid, payload.clone());
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 3;
        state.powers.set(PowerId::Parry, SlotWire::Int, 10);
        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.energy, 2, "the disjoint local-cost row remains live");
        assert_eq!(state.monsters[0].hp, 20);
        assert_eq!(state.block, 10);
        assert_eq!(state.card_states.get(source.uid), payload);
    }

    #[test]
    fn sovereign_blade_post_attack_block_refusals_restore_the_whole_play() {
        let (state, catalog, source) = blade_fixture(10);
        for listener in [false, true] {
            let mut case = state.clone();
            case.piles.get_mut(PileId::Play).make_mut().clear();
            case.piles.get_mut(PileId::Hand).make_mut().push(source);
            case.hp = 80;
            case.max_hp = 80;
            case.energy = 3;
            case.powers.set(PowerId::Parry, SlotWire::Int, 10);
            let expected = if listener {
                case.powers.set(PowerId::Accuracy, SlotWire::Int, 1);
                assert!(
                    case.fanouts
                        .set_after_block_gained_order(&[PowerId::Accuracy])
                );
                EngineRefusal::PowerHookNotModeled {
                    power: PowerId::Accuracy,
                    event: crate::hooks::HookEvent::AfterBlockGained,
                }
            } else {
                case.block = i32::MAX;
                EngineRefusal::MalformedArgs("player block")
            };
            let before = case.clone();
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 41 }];
            let before_events = events.clone();
            assert_eq!(
                crate::engine::play::play_card(
                    &mut case,
                    &catalog,
                    source.uid,
                    Some(0),
                    None,
                    &mut events,
                ),
                Err(expected)
            );
            assert_eq!(
                case, before,
                "the completed attack and play prefix roll back"
            );
            assert_eq!(events, before_events);
        }
    }

    #[test]
    fn sovereign_blade_refuses_a_missing_payload_before_mutation() {
        let (mut state, catalog, source) = blade_fixture(13);
        state.piles.get_mut(PileId::Play).make_mut()[0].flags = 0;
        let before = state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let target_identity = (state.monsters[0].slot, state.monsters[0].uid);
        let mut events = Vec::new();
        let before_events = events.clone();
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &[],
            events: &mut events,
        };
        let result =
            crate::engine::play::with_test_active_play_target(source.uid, target_identity, || {
                sovereign_blade_exact(&mut ctx)
            });
        assert!(matches!(
            result,
            Err(EngineRefusal::MalformedArgs(
                "sovereign_blade_exact physical source"
            ))
        ));
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    fn furnace_fixture(
        upgrade: u8,
    ) -> (
        HotState,
        crate::catalog::Catalog,
        HotCard,
        crate::catalog::CardSpec,
    ) {
        let mut builder = CatalogBuilder::new();
        let mut source_atom = None;
        for (id, level) in [
            (CardId::Furnace, 0),
            (CardId::Furnace, 1),
            (CardId::SovereignBlade, 0),
            (CardId::SovereignBlade, 1),
        ] {
            let atom = builder
                .intern(CardIdentity {
                    id,
                    upgrade: level,
                    enchantment: None,
                })
                .unwrap();
            if id == CardId::Furnace && level == upgrade {
                source_atom = Some(atom);
            }
        }
        let catalog = builder.build();
        assert!(furnace_catalog_is_exact(&catalog));
        let source = HotCard {
            uid: 71,
            atom: source_atom.unwrap(),
            flags: 0,
        };
        let spec = *catalog.spec(source.atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        state.next_card_uid = 72;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        // A live enemy keeps `damage_combat_is_ending` false: the card's
        // `PowerCmd.Apply` skips at IsEnding (#3218).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        (state, catalog, source, spec)
    }

    fn apply_furnace_fixture(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
        spec: &crate::catalog::CardSpec,
        events: &mut Vec<crate::engine::Event>,
    ) -> Result<(), EngineRefusal> {
        let step = catalog.steps(spec)[0];
        let args = catalog.args(step.args);
        crate::engine::play::with_test_active_play(source.uid, || {
            furnace_foundation_exact(&mut StepCtx {
                state,
                catalog,
                spec,
                source_uid: source.uid,
                target: None,
                selection: None,
                x_value: 0,
                args,
                events,
            })
        })
    }

    #[test]
    fn furnace_private_writer_applies_and_restacks_in_acquisition_order() {
        let (mut state, catalog, source, spec) = furnace_fixture(0);
        let mut events = Vec::new();
        apply_furnace_fixture(&mut state, &catalog, source, &spec, &mut events).unwrap();
        assert_eq!(state.powers.value(crate::ids::PowerId::Furnace), 5);
        assert_eq!(
            state.fanouts.after_side_turn_start_order(),
            &[crate::hot::AfterSideTurnStartToken::Furnace]
        );
        assert_eq!(
            events.last(),
            Some(&crate::engine::Event::PowerChanged {
                subject: crate::engine::Subject::Player,
                power: crate::ids::PowerId::Furnace,
                amount: 5,
            })
        );

        state.piles.get_mut(PileId::Play).make_mut()[0].atom = catalog
            .atom(&CardIdentity {
                id: CardId::Furnace,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let upgraded = state.piles.get(PileId::Play).as_slice()[0];
        let upgraded_spec = *catalog.spec(upgraded.atom).unwrap();
        apply_furnace_fixture(&mut state, &catalog, upgraded, &upgraded_spec, &mut events).unwrap();
        assert_eq!(state.powers.value(crate::ids::PowerId::Furnace), 12);
        assert_eq!(
            state.fanouts.after_side_turn_start_order(),
            &[crate::hot::AfterSideTurnStartToken::Furnace]
        );
    }

    #[test]
    fn furnace_public_play_covers_both_levels_and_replay_overflow_atomically() {
        for (upgrade, amount) in [(0, 5), (1, 7)] {
            let (mut state, catalog, source, _) = furnace_fixture(upgrade);
            state.piles.get_mut(PileId::Play).make_mut().clear();
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state.energy = 3;
            crate::engine::play::play_card(
                &mut state,
                &catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.powers.value(PowerId::Furnace), amount);
            assert_eq!(
                state.fanouts.after_side_turn_start_order(),
                [crate::hot::AfterSideTurnStartToken::Furnace]
            );
        }

        let (mut replay, catalog, source, _) = furnace_fixture(1);
        replay.piles.get_mut(PileId::Play).make_mut().clear();
        replay.piles.get_mut(PileId::Hand).make_mut().push(source);
        replay.energy = 3;
        replay
            .powers
            .set(PowerId::Furnace, SlotWire::Int, i32::MAX - 10);
        replay.powers.set(PowerId::SignalBoost, SlotWire::Int, 1);
        assert!(
            replay
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        let before = replay.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 19 }];
        let before_events = events.clone();
        assert_eq!(
            crate::engine::play::play_card(
                &mut replay,
                &catalog,
                source.uid,
                None,
                None,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("furnace amount"))
        );
        assert_eq!(replay, before, "the successful first Burst body rolls back");
        assert_eq!(events, before_events);
    }

    #[test]
    fn furnace_private_writer_refuses_malformed_power_order_and_overflow_atomically() {
        let (state, catalog, source, spec) = furnace_fixture(0);
        let mut cases = Vec::new();

        let mut wrong_wire = state.clone();
        wrong_wire
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Bool, 1);
        cases.push(("wrong wire", wrong_wire));

        let mut occupied_zero = state.clone();
        occupied_zero.powers = crate::powers::Slots::from_unsorted(vec![crate::powers::Slot {
            key: crate::ids::PowerId::Furnace,
            wire: SlotWire::Int,
            value: 0,
        }])
        .unwrap();
        cases.push(("occupied zero", occupied_zero));

        let mut negative = state.clone();
        negative
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, -1);
        cases.push(("negative", negative));

        let mut orphan = state.clone();
        assert!(
            orphan
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        cases.push(("orphan", orphan));

        let mut missing = state.clone();
        missing
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, 1);
        cases.push(("missing", missing));

        let mut duplicate = state.clone();
        duplicate
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, 1);
        assert!(duplicate.fanouts.set_after_side_turn_start_order(&[
            crate::hot::AfterSideTurnStartToken::Furnace,
            crate::hot::AfterSideTurnStartToken::Furnace,
        ]));
        cases.push(("duplicate", duplicate));

        let mut unknown = state.clone();
        unknown
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, 1);
        assert!(unknown.fanouts.set_after_side_turn_start_order(&[
            crate::hot::AfterSideTurnStartToken::BiasedCognition
        ]));
        cases.push(("unknown", unknown));

        let mut multiplayer = state.clone();
        multiplayer.multiplayer_ally_key = 1;
        cases.push(("multiplayer", multiplayer));

        let mut overflow = state.clone();
        overflow
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, i32::MAX);
        assert!(
            overflow
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        cases.push(("overflow", overflow));

        for (name, mut candidate) in cases {
            let before = candidate.clone();
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 9 }];
            let before_events = events.clone();
            assert!(
                apply_furnace_fixture(&mut candidate, &catalog, source, &spec, &mut events,)
                    .is_err(),
                "{name}"
            );
            assert_eq!(candidate, before);
            assert_eq!(events, before_events);
        }

        let mut same_hook_peer = state.clone();
        same_hook_peer
            .powers
            .set(crate::ids::PowerId::Coolant, SlotWire::Int, 1);
        assert!(
            same_hook_peer
                .fanouts
                .register_after_side_turn_start(crate::ids::PowerId::Coolant)
        );
        apply_furnace_fixture(
            &mut same_hook_peer,
            &catalog,
            source,
            &spec,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            same_hook_peer.fanouts.after_side_turn_start_order(),
            [
                crate::hot::AfterSideTurnStartToken::Coolant,
                crate::hot::AfterSideTurnStartToken::Furnace,
            ]
        );

        let mut disjoint = state;
        disjoint
            .powers
            .set(crate::ids::PowerId::Accuracy, SlotWire::Int, 1);
        apply_furnace_fixture(&mut disjoint, &catalog, source, &spec, &mut Vec::new()).unwrap();
        assert_eq!(disjoint.powers.value(crate::ids::PowerId::Furnace), 5);
    }

    #[test]
    fn furnace_private_state_uses_the_complete_type_one_listener_validator() {
        let (mut valid, _, _, _) = furnace_fixture(0);
        valid
            .powers
            .set(crate::ids::PowerId::Furnace, SlotWire::Int, 1);
        assert!(
            valid
                .fanouts
                .set_after_side_turn_start_order(&[crate::hot::AfterSideTurnStartToken::Furnace])
        );
        assert!(crate::engine::damage::player_type_one_listener_order_is_exact(&valid));
        assert!(furnace_private_state_is_exact(&valid));

        let mut hidden_monologue_tail = valid.clone();
        hidden_monologue_tail.fanouts.set_ruined_helmet_used();
        assert!(!furnace_private_state_is_exact(&hidden_monologue_tail));

        for listener in [
            crate::ids::PowerId::Vicious,
            crate::ids::PowerId::Shroud,
            crate::ids::PowerId::SleightOfFlesh,
        ] {
            let mut orphan_order = valid.clone();
            assert!(
                orphan_order
                    .fanouts
                    .set_after_power_amount_changed_order(&[listener])
            );
            assert!(!crate::engine::damage::player_type_one_listener_order_is_exact(&orphan_order));
            assert!(!furnace_private_state_is_exact(&orphan_order));

            let mut orphan_amount = valid.clone();
            orphan_amount.powers.set(listener, SlotWire::Int, 1);
            assert!(
                !crate::engine::damage::player_type_one_listener_order_is_exact(&orphan_amount)
            );
            assert!(!furnace_private_state_is_exact(&orphan_amount));

            let mut wrong_wire = valid.clone();
            wrong_wire.powers.set(listener, SlotWire::Bool, 1);
            assert!(
                wrong_wire
                    .fanouts
                    .set_after_power_amount_changed_order(&[listener])
            );
            assert!(!crate::engine::damage::player_type_one_listener_order_is_exact(&wrong_wire));
            assert!(!furnace_private_state_is_exact(&wrong_wire));
        }
    }

    #[test]
    fn furnace_private_writer_authenticates_source_catalog_and_public_refusal() {
        let (state, catalog, source, spec) = furnace_fixture(0);
        // #3221: `Furnace/<OnPlay>d__5::MoveNext` (RVA `0x3a0718`) only
        // applies FurnacePower (IL_00cc), so inexact piles apply exactly as
        // exact ones do and are left inexact.
        for upgrade in 0..=1 {
            let (exact, catalog, source, spec) = furnace_fixture(upgrade);
            let mut inexact = exact.clone();
            inexact.exact_piles = false;
            let mut exact_after = exact;
            let (mut exact_events, mut inexact_events) = (Vec::new(), Vec::new());
            apply_furnace_fixture(&mut exact_after, &catalog, source, &spec, &mut exact_events)
                .unwrap();
            apply_furnace_fixture(&mut inexact, &catalog, source, &spec, &mut inexact_events)
                .unwrap();
            assert_eq!(inexact_events, exact_events);
            assert!(!inexact.exact_piles, "a Furnace play reads no pile order");
            assert_eq!(
                inexact.powers.value(PowerId::Furnace),
                [5, 7][upgrade as usize]
            );
            inexact.exact_piles = true;
            assert_eq!(inexact, exact_after);
        }
        for mut candidate in {
            let mut duplicate_uid = state.clone();
            duplicate_uid
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(source);
            [duplicate_uid]
        } {
            let before = candidate.clone();
            let mut events = vec![crate::engine::Event::TurnEnded { turn: 4 }];
            let before_events = events.clone();
            assert!(
                apply_furnace_fixture(&mut candidate, &catalog, source, &spec, &mut events,)
                    .is_err()
            );
            assert_eq!(candidate, before);
            assert_eq!(events, before_events);
        }

        assert!(furnace_catalog_closure_is_exact(&catalog));
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::SovereignBlade,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some(),
            "interning any Furnace source must recursively close its fixed leaf"
        );

        let mut public_ctx_state = state;
        let mut public_events = Vec::new();
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args);
        let mut public_ctx = StepCtx {
            state: &mut public_ctx_state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args,
            events: &mut public_events,
        };
        crate::engine::play::with_test_active_play(source.uid, || furnace(&mut public_ctx))
            .unwrap();
        assert_eq!(public_ctx_state.powers.value(PowerId::Furnace), 5);
        assert!(IMPLEMENTED.contains(&StepKind::Furnace));
        assert!(crate::steps::is_implemented(StepKind::Furnace));
    }

    #[test]
    fn furnace_forge_terminal_entry_is_a_total_noop() {
        let (mut state, catalog, _, _) = furnace_fixture(0);
        state.piles.get_mut(PileId::Play).make_mut().clear();
        state.history.over = true;
        let before = state.clone();
        let mut events = vec![crate::engine::Event::TurnEnded { turn: 14 }];
        let before_events = events.clone();

        forge_exact(&mut state, &catalog, 5, &mut events).unwrap();

        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    fn hammer_party() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 10;
        state.next_card_uid = 100;
        // A live primary keeps the party out of the IsOverOrEnding window
        // that `ForgeCmd.Forge` returns on (#3112).
        state.monsters = std::sync::Arc::new(vec![HotMonster::new(MonsterKind::Toadpole, 30)]);
        state.multiplayer_ally_key = 1;
        assert!(state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        }));
        state
    }

    #[test]
    fn hammer_time_both_levels_install_unique_and_keep_sword_sage_implemented() {
        let mut builder = CatalogBuilder::new();
        let hammer_atoms = [0, 1].map(|upgrade| {
            builder
                .intern(CardIdentity {
                    id: CardId::HammerTime,
                    upgrade,
                    enchantment: None,
                })
                .unwrap()
        });
        let catalog = builder.build();
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::SovereignBlade,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some()
        );
        let mut state = hammer_party();
        for (index, atom) in hammer_atoms.iter().copied().enumerate() {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 10 + index as u32,
                atom,
                flags: 0,
            });
        }
        for uid in [10, 11] {
            crate::engine::play::play_card(&mut state, &catalog, uid, None, None, &mut Vec::new())
                .unwrap();
            assert!(state.fanouts.hammer_time(), "unique power stays amount one");
        }
        assert!(IMPLEMENTED.contains(&StepKind::HammerTimeExact));
        assert!(IMPLEMENTED.contains(&StepKind::SwordSage));

        let mut autoplay = hammer_party();
        autoplay.energy = 0;
        autoplay
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 12,
                atom: hammer_atoms[0],
                flags: 0,
            });
        crate::engine::play::autoplay_draw_top(&mut autoplay, &catalog, 1, &mut Vec::new())
            .unwrap();
        assert!(autoplay.fanouts.hammer_time());
        assert_eq!(autoplay.energy, 0, "direct AutoPlay spends no Energy");
    }

    #[test]
    fn hammer_time_forge_preserves_remote_pile_order_and_exact_damage() {
        let mut builder = CatalogBuilder::new();
        let blade_identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let blade_atom = builder.intern(blade_identity).unwrap();
        let catalog = builder.build();
        let mut state = hammer_party();
        state.fanouts.set_hammer_time(true);
        let local = HotCard {
            uid: 20,
            atom: blade_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        state.piles.get_mut(PileId::Hand).make_mut().push(local);
        state.card_states.set(
            local.uid,
            CardInstanceState {
                damage_growth: 10,
                ..CardInstanceState::default()
            },
        );
        let ordinary = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        {
            let ally = state.fanouts.multiplayer_ally_mut();
            ally.hand = std::sync::Arc::new(vec![
                MultiplayerAllyCard::sovereign_blade(blade_identity, 20).unwrap(),
            ]);
            ally.draw = std::sync::Arc::new(vec![
                MultiplayerAllyCard::sovereign_blade(blade_identity, 30).unwrap(),
            ]);
            ally.discard = std::sync::Arc::new(vec![
                MultiplayerAllyCard::immutable(ordinary).unwrap(),
                MultiplayerAllyCard::sovereign_blade(blade_identity, 40).unwrap(),
            ]);
        }
        forge_exact(&mut state, &catalog, 3, &mut Vec::new()).unwrap();
        assert_eq!(state.card_states.get(local.uid).damage_growth, 13);
        let ally = state.fanouts.multiplayer_ally();
        assert_eq!(ally.hand[0].sovereign_blade_damage(), Some(23));
        assert_eq!(ally.draw[0].sovereign_blade_damage(), Some(33));
        assert_eq!(
            ally.discard[0].identity, ordinary,
            "ordinary order is stable"
        );
        assert_eq!(ally.discard[1].sovereign_blade_damage(), Some(43));
    }

    #[test]
    fn hammer_time_remote_generation_redirects_and_late_failure_rolls_back() {
        let mut builder = CatalogBuilder::new();
        let blade_identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let blade_atom = builder.intern(blade_identity).unwrap();
        let catalog = builder.build();
        let ordinary = CardIdentity {
            id: CardId::DefendRegent,
            upgrade: 0,
            enchantment: None,
        };
        let mut generated = hammer_party();
        generated.fanouts.set_hammer_time(true);
        generated.fanouts.multiplayer_ally_mut().hand =
            std::sync::Arc::new(vec![MultiplayerAllyCard::immutable(ordinary).unwrap(); 10]);
        forge_exact(&mut generated, &catalog, 4, &mut Vec::new()).unwrap();
        let ally = generated.fanouts.multiplayer_ally();
        assert_eq!(ally.hand.len(), 10);
        assert_eq!(ally.discard[0].sovereign_blade_damage(), Some(14));
        assert_eq!(ally.owner_generated_cards_combat, 1);
        assert_eq!(
            generated.next_generated_hook_uid, 2,
            "local and remote generated Blades own one hook epoch each"
        );

        let mut existing = hammer_party();
        existing.fanouts.set_hammer_time(true);
        existing.next_generated_hook_uid = i32::MAX;
        let local = HotCard {
            uid: 30,
            atom: blade_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        existing.piles.get_mut(PileId::Hand).make_mut().push(local);
        existing.card_states.set(
            local.uid,
            CardInstanceState {
                damage_growth: 10,
                ..CardInstanceState::default()
            },
        );
        {
            let ally = existing.fanouts.multiplayer_ally_mut();
            ally.owner_generated_cards_combat = i32::MAX;
            ally.hand = std::sync::Arc::new(vec![
                MultiplayerAllyCard::sovereign_blade(blade_identity, 10).unwrap(),
            ]);
        }
        forge_exact(&mut existing, &catalog, 1, &mut Vec::new()).unwrap();
        assert_eq!(
            existing.fanouts.multiplayer_ally().hand[0].sovereign_blade_damage(),
            Some(11),
            "existing Blade skips generated counters"
        );

        let mut overflow = existing;
        overflow.fanouts.multiplayer_ally_mut().hand = std::sync::Arc::new(vec![
            MultiplayerAllyCard::sovereign_blade(blade_identity, i32::MAX).unwrap(),
        ]);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            forge_exact(&mut overflow, &catalog, 1, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "remote sovereign blade damage"
            ))
        );
        assert_eq!(
            overflow, before,
            "local Forge rolls back with remote suffix"
        );
        assert!(events.is_empty());
    }
}
