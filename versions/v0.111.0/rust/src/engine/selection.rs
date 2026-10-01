//! Exact generic card-selection interpreter and answer enumeration.

use crate::catalog::{CardSpec, Catalog, CompiledArg, RewardPool};
use std::cmp::Ordering;

use crate::hot::{
    CARD_FLAG_BOUND, CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_GENETIC_ALGORITHM_STATE,
    CARD_FLAG_HEXED, CARD_FLAG_PICK, CARD_FLAG_RINGING, CARD_FLAG_SOVEREIGN_BLADE_STATE,
    CardInstanceState, HotCard, HotState, PendingSelection, PendingSelectionKind, PileId,
};
use crate::ids::{FilterMode, SelectOp, StepKind, StepWord};

use super::draw::discard_and_draw;
use super::{EngineRefusal, Event, StepCtx};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SelectDisposition {
    Complete,
    Suspend,
}

thread_local! {
    /// Whether native's `CardSelectCmd` selector stack holds a
    /// `VakuuCardSelector` (#3414). See [`VakuuSelectorScope`].
    static VAKUU_SELECTOR_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whispering Earring's pushed `VakuuCardSelector` (#3414).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
///
/// - `WhisperingEarring/<AfterAutoPrePlayPhaseEnteredLate>d__8::MoveNext`
///   RVA `0x333fcc` calls `CardSelectCmd::PushSelector(new
///   VakuuCardSelector(), false)` (IL_0065-IL_006c) before its AutoPlay loop
///   and disposes the returned `StackedSelectorScope` in the loop's
///   `finally` (IL_0268-IL_0276), so every selection inside the loop,
///   `SpendResources` included, sees it.
/// - `CardSelectCmd::get_Selector` RVA `0x131889` peeks that stack. With a
///   selector, `CardSelectCmd/<FromHand>d__28::MoveNext` RVA `0x3e7568` and
///   `CardSelectCmd/<FromCombatPile>d__20::MoveNext` RVA `0x3e5e84` never
///   reserve a `PlayerChoiceSynchronizer` choice id (IL_007c / IL_0077
///   `brtrue`). The count-at-or-under-`MinSelect` auto-take still runs first
///   (FromHand IL_0166-IL_018d, FromCombatPile IL_015a-IL_017c). Otherwise
///   the selector receives the filtered options in live pile order. The only
///   reorder is FromCombatPile's Draw-pile view (IL_0191-IL_01dd).
/// - `VakuuCardSelector::GetSelectedCards` RVA `0x9d733` is
///   `options.Take(maxSelect).ToList()` (IL_0001-IL_0008), so the answer is
///   the first `MaxSelect` options.
///
/// Only the selector programs whose `CardSelectCmd` path was read above
/// resolve through the scope ([`vakuu_program_is_exact`], Glimmer). Every
/// other selection still suspends, which the Earring loop refuses by name.
pub(crate) struct VakuuSelectorScope {
    previous: bool,
}

impl VakuuSelectorScope {
    pub(crate) fn enter() -> Self {
        Self {
            previous: VAKUU_SELECTOR_ACTIVE.with(|active| active.replace(true)),
        }
    }
}

impl Drop for VakuuSelectorScope {
    fn drop(&mut self) {
        VAKUU_SELECTOR_ACTIVE.with(|active| active.set(self.previous));
    }
}

pub(crate) fn vakuu_selector_active() -> bool {
    VAKUU_SELECTOR_ACTIVE.with(std::cell::Cell::get)
}

/// Whether a generic select program resolves exactly under
/// [`VakuuSelectorScope`]: today only Cosmic Indifference (#3414).
///
/// `CosmicIndifference/<OnPlay>d__5::MoveNext` RVA `0x3950c0` builds
/// `CardSelectorPrefs(SelectionScreenPrompt, 1)` (IL_00aa-IL_00b0; the
/// two-argument constructor RVA `0x1397d4` passes the count as both
/// `MinSelect` and `MaxSelect`) and awaits the four-argument
/// `CardSelectCmd::FromCombatPile` over the Discard pile (IL_00c8-IL_00db,
/// `ldc.i4.3`). `<FromCombatPile>d__19::MoveNext` RVA `0x3e5d8c` forwards to
/// the filtered overload with the always-true `<>c::<FromCombatPile>b__19_0`
/// (IL_002e-IL_004d), so the Vakuu answer is the Discard pile's first card.
fn vakuu_program_is_exact(owner: &CardSpec, selector: Selector) -> bool {
    owner.identity.id == crate::ids::CardId::CosmicIndifference
        && matches!(owner.identity.upgrade, 0 | 1)
        && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
            == Some(owner.row)
        && crate::engine::play::body_enchantment_is_exact(owner)
        && selector.pile == PileId::Discard
        && selector.min == 1
        && selector.max == 1
        && selector.filter.is_none()
        && selector.operation
            == Operation::Move {
                destination: PileId::Draw,
                top: true,
            }
}

/// Whether Whispering Earring's AutoPlay of `spec` can meet a selection
/// only at the program [`VakuuSelectorScope`] resolves (#3414).
///
/// Glimmer (`FromHand`, `CardSelectorPrefs(prompt, PutBack)` with a null
/// filter at `Glimmer/<OnPlay>d__4::MoveNext` RVA `0x3a173c`
/// IL_00a7-IL_00d8) and Cosmic Indifference ([`vakuu_program_is_exact`]).
/// `draw_hooks_quiet` says no Draw hook can suspend (Stratagem, or
/// Hellraiser with a selecting Strike). Without it the answer is false:
/// those hooks' selections under the selector were not read here.
pub(crate) fn whispering_earring_child_selection_is_vakuu_resolved(
    spec: &CardSpec,
    catalog: &Catalog,
    draw_hooks_quiet: bool,
) -> bool {
    if !draw_hooks_quiet
        || !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return false;
    }
    match spec.identity.id {
        crate::ids::CardId::Glimmer => {
            crate::engine::admission::body_owned_select_program_is_supported(spec.row)
        }
        crate::ids::CardId::CosmicIndifference => match catalog.steps(spec) {
            [block, select] if block.kind == StepKind::Block && select.kind == StepKind::Select => {
                selector_from_args(catalog, spec, catalog.args(select.args))
                    .is_ok_and(|selector| vakuu_program_is_exact(spec, selector))
            }
            _ => false,
        },
        _ => false,
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Operation {
    /// Plural `CardCmd::Discard` of the answer, in pick order (Hidden
    /// Daggers, Prepared, and every one-card discard select). See
    /// [`selection_order_is_observable`] for the native order evidence.
    Discard,
    /// Serial `CardCmd::Exhaust` of the answer, in pick order.
    Exhaust,
    Upgrade,
    ApplyPermanentRetain,
    /// Sculpting Strike's `CardCmd.ApplyKeyword(card, Ethereal)` (#3022); see
    /// [`sculpting_strike_program_is_exact`].
    ApplyPermanentEthereal,
    ApplySingleTurnSly,
    Decisions,
    Move {
        destination: PileId,
        top: bool,
    },
    CloneGenerated {
        count: usize,
    },
    SeanceTransform,
    ChargeTransform {
        upgrade: u8,
    },
    RegentHandTransform {
        replacement: crate::ids::CardId,
        upgrade: u8,
    },
    /// Transfigure's exact-one Hand cost-and-replay writer (#3291); see
    /// [`transfigure_program_is_exact`] and [`apply_transfigure`].
    Transfigure,
}

#[derive(Copy, Clone, Debug)]
struct Selector {
    pile: PileId,
    min: usize,
    max: usize,
    filter: Option<FilterMode>,
    operation: Operation,
}

fn selector(ctx: &StepCtx<'_>) -> Result<Selector, EngineRefusal> {
    selector_from_args(ctx.catalog, ctx.spec, ctx.args)
}

/// Whether this is the exact canonical Sculpting Strike L0/L1 select
/// program: one Hand card without the Ethereal keyword gains it (#3022).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `SculptingStrike/<OnPlay>d__7::MoveNext` RVA `0x3b8cec` awaits
/// `CardSelectCmd::FromHand` (IL_0115, `CardSelectorPrefs` count 1 at
/// IL_00ef) with the `<>c::<OnPlay>b__7_0` filter, then for the selected card
/// builds a one-element `CardKeyword[]` holding `2` (Ethereal, IL_0187) and
/// calls `CardCmd::ApplyKeyword` (IL_0189), which is `CardModel::AddKeyword`
/// per element (`CardCmd::ApplyKeyword` RVA `0x12fd98` IL_0018) into the
/// instance's `LocalKeywords` (`CardModel::AddKeyword` RVA `0x7d4f6`
/// IL_0008). The filter `<>c::<OnPlay>b__7_0` RVA `0x3b8cda` is
/// `!GetKeywordsWithSources(2).Contains(Ethereal)`; with flag 2 and not flag
/// 4, `CardModel::GetKeywordsWithSources` RVA `0x7cbd0` returns
/// `LocalKeywords` (IL_0041..IL_0054) — canonical keywords plus added ones
/// (`get_LocalKeywords` RVA `0x7cb91` UnionWith `CanonicalKeywords`) — and
/// skips `Hook::ModifyKeywordsInCombat`, so a Hex-granted Ethereal does not
/// exclude a card. That is [`FilterMode::WithoutEtherealKeyword`]. The frozen
/// registry carried Snap's Retain step here instead; Snap
/// (`<OnPlay>d__9` RVA `0x3bc99c`, IL_01ac `ldc.i4.5`) is the Retain writer.
fn sculpting_strike_program_is_exact(
    owner: &CardSpec,
    pile: PileId,
    min: usize,
    max: usize,
    filter: Option<FilterMode>,
) -> bool {
    owner.identity.id == crate::ids::CardId::SculptingStrike
        && matches!(owner.identity.upgrade, 0 | 1)
        && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
            == Some(owner.row)
        && crate::engine::play::body_enchantment_is_exact(owner)
        && pile == PileId::Hand
        && min == 1
        && max == 1
        && filter == Some(FilterMode::WithoutEtherealKeyword)
}

/// Whether this is the exact canonical Transfigure L0/L1 select program: one
/// unfiltered Hand card gains `+1` Energy cost for the combat and one
/// `BaseReplayCount` (#3291).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `Transfigure/<OnPlay>d__9::MoveNext` RVA `0x3c4438` builds
/// `CardSelectorPrefs(SelectionScreenPrompt, 1)` (IL_0031-IL_0037, the same
/// exact-one prefs Sculpting Strike and Armaments pass), a `null` filter
/// (IL_003c), and awaits `CardSelectCmd::FromHand` (IL_003e). The row is
/// otherwise static: `Transfigure::.ctor` RVA `0xef171` fixes Energy cost 1 /
/// Skill / Rare / `Self`; `get_CanonicalKeywords` RVA `0xef181` is the single
/// keyword `1` (Exhaust), which `OnUpgrade` RVA `0xef207` removes and nothing
/// else; `get_CanBeGeneratedInCombat` RVA `0xef17e` returns false. Registry
/// equality pins both levels' complete immutable rows, so a forged owner,
/// pile, count or filter stays `"select operation"`.
fn transfigure_program_is_exact(
    owner: &CardSpec,
    pile: PileId,
    min: usize,
    max: usize,
    filter: Option<FilterMode>,
) -> bool {
    owner.identity.id == crate::ids::CardId::Transfigure
        && matches!(owner.identity.upgrade, 0 | 1)
        && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
            == Some(owner.row)
        && crate::engine::play::body_enchantment_is_exact(owner)
        && pile == PileId::Hand
        && min == 1
        && max == 1
        && filter.is_none()
}

/// Whether this is the exact canonical Dual Wield L0/L1 select program: one
/// Hand Attack-or-Power card is cloned `1 + upgrade` times into Hand/Bottom
/// (#3374).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
///
/// - `DualWield::.ctor` RVA `0xde4c5` is cost 1 / Skill (IL_0003 `ldc.i4.2`)
///   / Self; `get_CanonicalVars` RVA `0xde4d2` is one `CardsVar(1)`
///   (IL_0001-IL_0002) and `OnUpgrade` RVA `0xde533` is
///   `Cards.UpgradeValueBy(1)` (IL_0007-IL_0011): one clone at L0, two at L1.
/// - `DualWield/<OnPlay>d__5::MoveNext` RVA `0x39aa54` builds
///   `CardSelectorPrefs(SelectionScreenPrompt, 1)` (IL_0027-IL_002d, the same
///   exact-one prefs Heirloom Hammer and Armaments pass) and awaits
///   `CardSelectCmd::FromHand` (IL_005f) with `<>c::<OnPlay>b__5_0`
///   RVA `0x39aa30`, which is `Type == 1 (Attack) || Type == 3 (Power)`
///   (IL_000d-IL_0019). `CardModel::get_Type` RVA `0x7c864` reads the
///   constructor-set `<Type>k__BackingField` (no `CardModel::set_Type`
///   exists), so the row's static card type is the filter's input:
///   [`FilterMode::AttackOrPower`].
/// - `FirstOrDefault` of the result (IL_00c3) is null when nothing was
///   selectable, and IL_00d3 `brfalse` then returns with no clone: the zero
///   candidate case is a no-op, like `CardSelectCmd::FromHand`'s one-candidate
///   auto-take is an ordinary answer.
/// - Otherwise the loop IL_00e4-IL_0181 runs `i < DynamicVars.Cards.IntValue`
///   (re-read each iteration; nothing in the body writes it) and on each pass
///   calls `CardModel::CreateClone` on the SELECTED card (IL_00ea, RVA
///   `0x7e1e0`: `CardScope.CloneCard`, `_cloneOf`, and
///   `ExhaustOnNextPlay = false` — a play-transient flag Rust never parks on a
///   Hand card) and awaits `CardPileCmd::AddGeneratedCardToCombat(clone,
///   PileType 2 /* Hand */, Owner, position 1 /* Bottom */)` (IL_00f1-IL_00fb,
///   RVA `0x1305a0`) — the exact argument tuple Heirloom Hammer's body passes
///   at IL_01b3-IL_01bd. Each clone is therefore its own generated-card
///   command, redirected to Discard by the Add when Hand is full, and the
///   source is never removed. [`apply`] re-reads the live source before every
///   pass so a clone always copies the selected card's current state.
fn dual_wield_program_is_exact(
    owner: &CardSpec,
    pile: PileId,
    min: usize,
    max: usize,
    filter: Option<FilterMode>,
    count: i64,
) -> bool {
    owner.identity.id == crate::ids::CardId::DualWield
        && matches!(owner.identity.upgrade, 0 | 1)
        && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
            == Some(owner.row)
        && crate::engine::play::body_enchantment_is_exact(owner)
        && pile == PileId::Hand
        && min == 1
        && max == 1
        && filter == Some(FilterMode::AttackOrPower)
        && count == i64::from(owner.identity.upgrade) + 1
}

fn selector_from_args(
    catalog: &Catalog,
    owner: &CardSpec,
    args: &[CompiledArg],
) -> Result<Selector, EngineRefusal> {
    let [
        CompiledArg::Pile(pile),
        CompiledArg::I(min),
        CompiledArg::I(max),
        filter,
        operation,
    ] = args
    else {
        return Err(EngineRefusal::MalformedArgs("select"));
    };
    let min: usize = (*min)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("select min"))?;
    let max: usize = (*max)
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("select max"))?;
    if min > max {
        return Err(EngineRefusal::MalformedArgs("select range"));
    }
    let filter = match filter {
        CompiledArg::Nil => None,
        CompiledArg::Filter(filter) => Some(*filter),
        _ => return Err(EngineRefusal::MalformedArgs("select filter")),
    };
    if matches!(filter, Some(FilterMode::WithoutEtherealKeyword))
        && owner.identity.id != crate::ids::CardId::SculptingStrike
        || matches!(filter, Some(FilterMode::Skill))
            && owner.identity.id != crate::ids::CardId::SecretTechnique
        || matches!(filter, Some(FilterMode::Attack))
            && owner.identity.id != crate::ids::CardId::SecretWeapon
    {
        // `_run_steps_inner` (frozen Python, deleted #2827) binds these two Batch-65 native filter
        // readers to their source rows; a forged reuse is an I5 refusal.
        return Err(EngineRefusal::MalformedArgs("select filter owner"));
    }
    let operation = match operation {
        CompiledArg::Select(SelectOp::Discard | SelectOp::DiscardAll) => Operation::Discard,
        CompiledArg::Select(SelectOp::Exhaust) => Operation::Exhaust,
        CompiledArg::Select(SelectOp::Upgrade)
            if owner.identity.id == crate::ids::CardId::Armaments
                && owner.identity.upgrade == 0
                && crate::engine::play::body_enchantment_is_exact(owner)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row)
                && *pile == PileId::Hand
                && min == 1
                && max == 1
                && filter == Some(FilterMode::Upgradable) =>
        {
            // Current v0.111.0 `Armaments/<OnPlay>d__5::MoveNext` RVA
            // 0x38a310 binds this one-card CardCmd::Upgrade consumer to the
            // exact L0 row. Other Upgrade selectors stay capability-dark.
            Operation::Upgrade
        }
        CompiledArg::Word(StepWord::ApplySingleTurnSly)
            if owner.identity.id == crate::ids::CardId::HandTrick
                && matches!(owner.identity.upgrade, 0 | 1)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Hand
                && min == 1
                && max == 1
                && filter == Some(FilterMode::SkillWithoutSlyThisTurn) =>
        {
            // HandTrick.OnPlay MoveNext 0x3a38ec: select one Skill without
            // IsSlyThisTurn, then ApplySingleTurnSly to that physical card.
            Operation::ApplySingleTurnSly
        }
        CompiledArg::Word(StepWord::ApplyPermanentRetain) => Operation::ApplyPermanentRetain,
        CompiledArg::Word(StepWord::TransfigureExact)
            if transfigure_program_is_exact(owner, *pile, min, max, filter) =>
        {
            Operation::Transfigure
        }
        CompiledArg::Word(StepWord::ApplyPermanentEthereal)
            if sculpting_strike_program_is_exact(owner, *pile, min, max, filter) =>
        {
            Operation::ApplyPermanentEthereal
        }
        CompiledArg::List(span) => match catalog.args(*span) {
            [
                CompiledArg::Word(StepWord::DecisionsReplayExact),
                CompiledArg::I(3),
            ] if matches!(owner.identity.id, crate::ids::CardId::DecisionsDecisions)
                && matches!(owner.identity.upgrade, 0 | 1)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Hand
                && min == 1
                && max == 1
                && filter == Some(FilterMode::SkillWithoutUnplayable) =>
            {
                Operation::Decisions
            }
            [
                CompiledArg::Word(StepWord::FixedTransform),
                CompiledArg::Word(StepWord::Seance),
            ] if crate::engine::admission::seance_program_is_supported(owner.row)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Draw
                && min == 1
                && max == 1
                && filter.is_none() =>
            {
                Operation::SeanceTransform
            }

            [
                CompiledArg::Word(StepWord::FixedTransform),
                CompiledArg::Word(StepWord::Charge),
            ] if crate::engine::admission::charge_program_is_supported(owner.row)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Draw
                && min == 2
                && max == 2
                && filter.is_none() =>
            {
                Operation::ChargeTransform {
                    upgrade: owner.identity.upgrade,
                }
            }

            [
                CompiledArg::Word(StepWord::FixedTransform),
                CompiledArg::Word(word),
            ] if crate::engine::admission::regent_hand_transform_program_is_supported(
                owner.row,
            ) && crate::content_tables::card_row(
                owner.identity.id,
                owner.identity.upgrade,
            ) == Some(owner.row)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Hand
                && filter.is_none()
                && matches!(
                    (owner.identity.id, *word, min, max),
                    (crate::ids::CardId::Begone, StepWord::Begone, 1, 1)
                        | (crate::ids::CardId::Guards, StepWord::Guards, 0, 999_999_999)
                ) =>
            {
                Operation::RegentHandTransform {
                    replacement: if owner.identity.id == crate::ids::CardId::Begone {
                        crate::ids::CardId::MinionStrike
                    } else {
                        crate::ids::CardId::MinionSacrifice
                    },
                    upgrade: owner.identity.upgrade,
                }
            }

            [
                CompiledArg::Select(SelectOp::Move),
                CompiledArg::Word(destination),
                CompiledArg::Word(position),
            ] => {
                let destination = match destination {
                    StepWord::Hand => PileId::Hand,
                    StepWord::Draw => PileId::Draw,
                    _ => return Err(EngineRefusal::MalformedArgs("select move destination")),
                };
                let top = match position {
                    StepWord::Bottom if destination == PileId::Hand => false,
                    StepWord::Top if destination == PileId::Draw => true,
                    _ => return Err(EngineRefusal::MalformedArgs("select move position")),
                };
                Operation::Move { destination, top }
            }
            [
                CompiledArg::Word(StepWord::CloneGenerated),
                CompiledArg::I(1),
            ] if owner.identity.id == crate::ids::CardId::HeirloomHammer
                && matches!(owner.identity.upgrade, 0 | 1)
                && crate::engine::play::body_enchantment_is_exact(owner)
                && *pile == PileId::Hand
                && min == 1
                && max == 1
                && filter == Some(FilterMode::CurrentBuildColorless)
                && crate::content_tables::card_row(owner.identity.id, owner.identity.upgrade)
                    == Some(owner.row) =>
            {
                // Current-v0.111 DLL SHA-256
                // `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
                // Heirloom Hammer's selector lambda (RVA 0x3a46c2) and
                // OnPlay body (RVA 0x3a46d0) bind this
                // opaque writer to its exact canonical source row. Dual Wield
                // carries the same word with a different filter/count and is
                // authenticated by its own owner seam below
                // ([`dual_wield_program_is_exact`], #3374).
                Operation::CloneGenerated { count: 1 }
            }
            [
                CompiledArg::Word(StepWord::CloneGenerated),
                CompiledArg::I(count),
            ] if dual_wield_program_is_exact(owner, *pile, min, max, filter, *count) => {
                Operation::CloneGenerated {
                    count: usize::from(owner.identity.upgrade) + 1,
                }
            }
            _ => return Err(EngineRefusal::MalformedArgs("select operation")),
        },
        // Every other Upgrade shape needs an independently authenticated
        // owner and catalog-wide next-level closure. Opaque operations need
        // their own physical-card writers. Admission keeps both out.
        _ => return Err(EngineRefusal::MalformedArgs("select operation")),
    };
    Ok(Selector {
        pile: *pile,
        min,
        max,
        filter,
        operation,
    })
}

pub(crate) fn exhaust_selector_pile(
    catalog: &Catalog,
    owner: &CardSpec,
    args: &[CompiledArg],
) -> Option<PileId> {
    let selector = selector_from_args(catalog, owner, args).ok()?;
    matches!(selector.operation, Operation::Exhaust).then_some(selector.pile)
}

/// `card.VisualCardPool.IsColorless`, Heirloom Hammer's hand filter
/// (`HeirloomHammer/<>c::<OnPlay>b__3_0` RVA `0x3a46c2` IL_0002-IL_0007).
///
/// Native authority: v0.111.0 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `CardModel::get_VisualCardPool` (RVA `0x7c91e`) is `get_Pool`, the pool
/// whose membership holds the id (`CardModel::get_Pool` RVA `0x7c878`). Ten
/// Event cards override it with a character pool: Clash, Dual Wield and
/// Entrench `ModelDb.CardPool<IroncladCardPool>` (MethodSpec `0x2b000e80`),
/// Caltrops, Distraction and Outmaneuver `<SilentCardPool>` (`0x2b000e82`),
/// Hello World, Rebound, Rip and Tear and Stack `<DefectCardPool>`
/// (`0x2b000e83`). `get_IsColorless` is true for exactly `ColorlessCardPool`
/// (RVA `0xf119c`), `DeprecatedCardPool` (`0xf1a07`), `EventCardPool`
/// (`0xf1b1d`) and `TokenCardPool` (`0xf2e5b`); every character pool and
/// `Curse`/`Quest`/`Status` return false.
///
/// The generated row's `pool` field is not this: it names the pool only for
/// some rows (Thinking Ahead, a Colorless-pool card, has none — census
/// 70022CD6G3J0 node 27 copied it natively).
fn visual_card_pool_is_colorless(id: crate::ids::CardId) -> bool {
    use crate::ids::CardId;
    if matches!(
        id,
        CardId::Clash
            | CardId::DualWield
            | CardId::Entrench
            | CardId::Caltrops
            | CardId::Distraction
            | CardId::Outmaneuver
            | CardId::HelloWorld
            | CardId::Rebound
            | CardId::RipAndTear
            | CardId::Stack
    ) || crate::catalog::card_character_pool(id).is_some()
    {
        return false;
    }
    if crate::content_tables::COLORLESS_CARD_POOL_MEMBERSHIP_V1110.contains(&id) {
        return true;
    }
    crate::content_tables::SHARED_CARD_POOL_MEMBERSHIP_V1110
        .iter()
        .find(|(_, members)| members.contains(&id))
        .is_some_and(|(pool, _)| matches!(*pool, "DEPRECATED" | "EVENT" | "TOKEN"))
}

fn matches_filter(
    state: &HotState,
    card: HotCard,
    catalog: &Catalog,
    filter: FilterMode,
) -> Result<bool, EngineRefusal> {
    let spec = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    Ok(match filter {
        FilterMode::Attack => spec.is_attack,
        FilterMode::AttackOrPower => spec.is_attack || spec.is_power,
        FilterMode::CurrentBuildColorless => visual_card_pool_is_colorless(spec.identity.id),
        FilterMode::Skill => spec.is_skill,
        FilterMode::SkillWithoutSlyThisTurn => {
            spec.is_skill && !state.card_states.get(card.uid).is_sly(spec.sly)
        }
        // DecisionsDecisions predicate 0x3977a2 reads Keywords.Unplayable,
        // not the broader runtime CanPlay predicate.
        FilterMode::SkillWithoutUnplayable => spec.is_skill && !spec.native_unplayable,
        FilterMode::WithoutEffectiveRetain => {
            !crate::engine::cards::effective_retain(state, spec, card.uid)
        }
        // Sculpting Strike `<>c::<OnPlay>b__7_0` RVA 0x3b8cda:
        // `LocalKeywords` = the row's canonical keywords plus AddKeyword
        // writes; Hex's combat-time Ethereal is not read (see
        // `sculpting_strike_program_is_exact`).
        FilterMode::WithoutEtherealKeyword => {
            !(spec.ethereal || state.card_states.get(card.uid).local_ethereal())
        }
        FilterMode::Upgradable => {
            if !crate::engine::cards::native_card_is_upgradable(spec.identity) {
                false
            } else {
                let next_identity = crate::catalog::CardIdentity {
                    id: spec.identity.id,
                    upgrade: spec
                        .identity
                        .upgrade
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("CardCmd::Upgrade level"))?,
                    enchantment: spec.identity.enchantment,
                };
                catalog
                    .atom(&next_identity)
                    .ok_or(EngineRefusal::UnknownMintIdentity(next_identity))?;
                true
            }
        }
    })
}

fn candidates(
    state: &HotState,
    catalog: &Catalog,
    selector: Selector,
) -> Result<Vec<HotCard>, EngineRefusal> {
    if state.history.over {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for card in state.piles.get(selector.pile).as_slice() {
        if catalog.spec(card.atom).is_none() {
            return Err(EngineRefusal::UnknownAtom(card.atom));
        }
        if match selector.filter {
            Some(filter) => matches_filter(state, *card, catalog, filter)?,
            None => true,
        } {
            out.push(*card);
        }
    }
    Ok(out)
}

fn sorted_exact_candidates(
    state: &HotState,
    catalog: &Catalog,
    selector: Selector,
) -> Result<Vec<HotCard>, EngineRefusal> {
    let mut out = candidates(state, catalog, selector)?;
    if let Some(card) = out.iter().find(|card| catalog.spec(card.atom).is_none()) {
        return Err(EngineRefusal::UnknownAtom(card.atom));
    }
    let order = candidate_order_is_label_only(selector);
    insertion_sort_payloads(state, catalog, &mut out, order)?;
    Ok(out)
}

/// How [`card_payload_cmp_in`] treats a slot Python cannot order (`None`
/// against a tuple, #2985).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayloadOrder {
    /// Mirror Python's tuple comparison, refusals included.
    PythonTuple,
    /// Order `None` before `Some` in the optional enchantment, enchantment-
    /// state and Sovereign Blade slots. Sound only where the candidate order
    /// is a label and never reaches game state; see
    /// [`candidate_order_is_label_only`].
    LabelOnly,
}

/// Whether a generic selector's candidate order is only the positional label
/// of its answers, so any deterministic total order is exact (#2985).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// never compares the candidates of a combat-pile pick against each other.
/// `CardSelectCmd/<FromCombatPile>d__20::MoveNext` RVA `0x3e5e84` filters the
/// pile in pile order (`Where`/`ToList`, IL_0120-IL_0136); its only reorder is
/// the automated-selector view of the **Draw** pile (`get_Type` == 1 at
/// IL_0191-IL_0197, `OrderBy` rarity / `ThenBy` ModelId at IL_01b9/IL_01dd,
/// and again at IL_02b8/IL_02dc for the local selector), a display order that
/// cannot see an enchantment or an affliction either. The value it returns is
/// the player's own pick list (`PlayerChoiceResult::AsCombatCards`,
/// IL_04c3), unsorted. `CardModel::CompareTo` RVA `0x7e3d0` — model id via
/// `AbstractModel::CompareTo` (IL_0019), then `CurrentUpgradeLevel`
/// (IL_002c-IL_003a), else 0 (IL_0045) — is not called on this path, and it
/// would tie an enchanted card with its plain twin anyway. Headbutt
/// (`<OnPlay>d__3::MoveNext` RVA `0x3a416c`) passes the Discard pile
/// (IL_00fa `ldc.i4.3`) to `FromCombatPile` (IL_010d) and moves
/// `FirstOrDefault` of the answer (IL_0170) with `CardPileCmd::Add`
/// (IL_0181).
///
/// So the Python payload sort is an enumeration canonicalizer, not native
/// semantics. It would reach game state only through the internal order of a
/// multi-card sub-multiset handed to a sink that does not fan out its pick
/// orders; since #2524 no admitted multi-pick program has one (the
/// discard/exhaust family is order-observable). Where every answer is a single card
/// (`max <= 1`), or where [`selection_order_is_observable`] fans each subset
/// out into all of its pick orders, the option SET is independent of the
/// candidate order and the order only numbers the options. Everywhere else
/// the Python refusal stands.
fn candidate_order_is_label_only(selector: Selector) -> PayloadOrder {
    if selector.max <= 1 || selection_order_is_observable(selector.operation) {
        PayloadOrder::LabelOnly
    } else {
        PayloadOrder::PythonTuple
    }
}

/// Sort exact physical candidates by Python's uid-free `PhysicalCardPick`
/// tuple payload while preserving source order for equal payloads.
///
/// Replay and generic selectors both feed this ordering into `_pick_subsets`.
/// Keep the comparison fallible: Python refuses tuple shapes such as equal-GA
/// growth with `None` versus integer deck rows, so the port must not invent an
/// order merely to enumerate actions.
pub(crate) fn sorted_physical_card_pick_payloads(
    state: &HotState,
    catalog: &Catalog,
    mut out: Vec<HotCard>,
) -> Result<Vec<HotCard>, EngineRefusal> {
    sort_physical_card_pick_payloads_in_place(state, catalog, &mut out)?;
    Ok(out)
}

/// In-place form used by caller-owned legal-action scratch storage.
pub(crate) fn sort_physical_card_pick_payloads_in_place(
    state: &HotState,
    catalog: &Catalog,
    out: &mut [HotCard],
) -> Result<(), EngineRefusal> {
    if let Some(card) = out.iter().find(|card| catalog.spec(card.atom).is_none()) {
        return Err(EngineRefusal::UnknownAtom(card.atom));
    }
    insertion_sort_payloads(state, catalog, out, PayloadOrder::PythonTuple)
}

fn insertion_sort_payloads(
    state: &HotState,
    catalog: &Catalog,
    out: &mut [HotCard],
    order: PayloadOrder,
) -> Result<(), EngineRefusal> {
    // Python sorts Counter(PhysicalCardPick) rows by the complete uid-free
    // tuple payload. Its uid-sensitive equality keeps equal-payload siblings
    // separate, while the stable sort keeps their source-pile order. Use an
    // insertion sort so Python-incomparable tuple shapes can refuse instead
    // of being hidden inside an infallible comparator.
    for right in 1..out.len() {
        let mut index = right;
        while index > 0
            && card_payload_cmp_in(state, catalog, out[index], out[index - 1], order)?
                == Ordering::Less
        {
            out.swap(index, index - 1);
            index -= 1;
        }
    }
    Ok(())
}

/// StableShuffle input for Seeker Strike, ordered by native
/// `CardModel.CompareTo` before the Selection-stream shuffle.
pub(crate) fn seeker_sorted_draw(
    state: &HotState,
    catalog: &Catalog,
) -> Result<Vec<HotCard>, EngineRefusal> {
    // Unlike the later FromCombatPile selector, StableShuffle is not ending-
    // gated: a Seeker attack that kills the final enemy still sorts and
    // consumes the complete shuffle before the selector observes IsEnding.
    let draw = state.piles.get(PileId::Draw).as_slice();
    if let Some(card) = draw.iter().find(|card| catalog.spec(card.atom).is_none()) {
        return Err(EngineRefusal::UnknownAtom(card.atom));
    }
    Ok(crate::dotnet_sort::dotnet_list_sort_by_key(draw, |card| {
        let spec = catalog
            .spec(card.atom)
            .expect("Seeker Draw atoms were validated above");
        (spec.identity.id as u16, spec.identity.upgrade)
    }))
}

fn snapshot_matches_live(live: HotCard, frozen: HotCard) -> bool {
    live.uid == frozen.uid
        && live.atom == frozen.atom
        && (live.flags & !CARD_FLAG_PICK) == (frozen.flags & !CARD_FLAG_PICK)
}

fn purity_candidates<'a>(
    state: &HotState,
    selection_cards: &'a [HotCard],
) -> Result<&'a [HotCard], EngineRefusal> {
    let live = state.piles.get(PileId::Hand).as_slice();
    if live.len() != selection_cards.len()
        || !live
            .iter()
            .zip(selection_cards.iter())
            .all(|(live, frozen)| snapshot_matches_live(*live, *frozen))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(selection_cards)
}

fn seeker_candidates(
    state: &HotState,
    selection_cards: &[HotCard],
) -> Result<Vec<HotCard>, EngineRefusal> {
    if selection_cards.is_empty() || selection_cards.len() > 3 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut out = Vec::with_capacity(selection_cards.len());
    for live in state.piles.get(PileId::Draw).as_slice() {
        if let Some(frozen) = selection_cards.iter().find(|frozen| frozen.uid == live.uid) {
            if !snapshot_matches_live(*live, *frozen) {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            out.push(*live);
        }
    }
    if out.len() != selection_cards.len() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(out)
}

fn abundance_candidates<'a>(
    state: &HotState,
    catalog: &Catalog,
    selection_cards: &'a [HotCard],
) -> Result<&'a [HotCard], EngineRefusal> {
    // #3122: the same owner-pool gate as `steps::neutral::begin_abundance_exact`
    // (`Abundance/<OnPlay>d__6::MoveNext` `0x3888ec` reads no Entropy owner).
    // #3285: and the same pool — the owner's recorded-profile Power projection
    // (`0x3888ec` IL_0027-IL_007c: `GetUnlockedCards(Owner.UnlockState, ..)`
    // then `Where(Type == Power)`), so a partial profile resolves too.
    if selection_cards.len() != 3
        || state
            .entropy_card_pool
            .is_some_and(|pool| Some(pool) != state.reward_card_pool)
        || !crate::engine::cards::unlock_profile_is_recorded(state, catalog)
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let Some(owner) = state.reward_card_pool else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    let pool = catalog.owner_type_generation_pool(owner, crate::content_tables::CardType::Power);
    let mut ids = std::collections::BTreeSet::new();
    for card in selection_cards {
        if card.uid != 0 || card.flags != 0 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let spec = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if spec.identity.upgrade != 1
            || spec.identity.enchantment.is_some()
            || !spec.is_power
            || !pool.contains(&spec.identity.id)
            || !ids.insert(spec.identity.id)
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    Ok(selection_cards)
}

fn discovery_candidates<'a>(
    state: &HotState,
    catalog: &Catalog,
    active: HotCard,
    selection_cards: &'a [HotCard],
) -> Result<&'a [HotCard], EngineRefusal> {
    let source = catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
    let Some(owner) = state.reward_card_pool else {
        return Err(EngineRefusal::ContinuationNotModeled);
    };
    let pool = catalog.owner_generation_pool(owner);
    if selection_cards.len() != 3
        || source.identity.id != crate::ids::CardId::Discovery
        || !matches!(source.identity.upgrade, 0 | 1)
        || state
            .entropy_card_pool
            .is_some_and(|pool| Some(pool) != state.reward_card_pool)
        || !super::cards::unlock_profile_is_recorded(state, catalog)
        || state.multiplayer_ally_key != 0
        || (owner == RewardPool::Regent && !state.spectrum_shift_generation_pool())
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut ids = std::collections::BTreeSet::new();
    for card in selection_cards {
        if card.uid != 0 || card.flags != 0 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let option = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if option.identity.upgrade != 0
            || option.identity.enchantment.is_some()
            || !pool.contains(&option.identity.id)
            || !ids.insert(option.identity.id)
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    Ok(selection_cards)
}

fn splash_candidates<'a>(
    state: &HotState,
    catalog: &Catalog,
    active: HotCard,
    selection_cards: &'a [HotCard],
) -> Result<&'a [HotCard], EngineRefusal> {
    let source = catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
    let owner = state
        .reward_card_pool
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let pool = catalog.splash_attack_pool(owner);
    if selection_cards.len() != 3
        || source.identity.id != crate::ids::CardId::Splash
        || !matches!(source.identity.upgrade, 0 | 1)
        // A partial profile is exact provenance for Splash alone (#2469).
        || !(state.fully_unlocked_card_pool_epochs || catalog.splash_unlock_epochs().is_some())
        || state.multiplayer_ally_key != 0
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut ids = std::collections::BTreeSet::new();
    for card in selection_cards {
        if card.uid != 0 || card.flags != 0 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let option = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if option.identity.upgrade != source.identity.upgrade
            || option.identity.enchantment.is_some()
            || !pool.contains(&option.identity.id)
            || !ids.insert(option.identity.id)
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    Ok(selection_cards)
}

fn quasar_candidates<'a>(
    state: &HotState,
    catalog: &Catalog,
    active: HotCard,
    selection_cards: &'a [HotCard],
) -> Result<&'a [HotCard], EngineRefusal> {
    let source = catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
    if selection_cards.len() != 3
        || source.identity.id != crate::ids::CardId::Quasar
        || !matches!(source.identity.upgrade, 0 | 1)
        || !super::cards::colorless_generation_provenance_is_exact(state, catalog)
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // #2560: the recorded profile's Colorless pool, the one `begin_quasar_exact`
    // drew the three options from.
    let pool = catalog.colorless_generation_pool();
    let mut ids = std::collections::BTreeSet::new();
    for card in selection_cards {
        if card.uid != 0 || card.flags != 0 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let option = catalog
            .spec(card.atom)
            .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
        if option.identity.upgrade != source.identity.upgrade
            || option.identity.enchantment.is_some()
            || !pool.contains(&option.identity.id)
            || !ids.insert(option.identity.id)
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
    }
    Ok(selection_cards)
}

fn hand_cap_selector(
    state: &HotState,
    spec: &CardSpec,
    next_step: u32,
    selection_amount: i32,
    selection_cards: &[HotCard],
) -> Result<Selector, EngineRefusal> {
    let space = super::draw::MAX_CARDS_IN_HAND.saturating_sub(state.piles.get(PileId::Hand).len());
    let selection = crate::steps::hand_cap::pending_selection_for_row(spec.row, space)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if next_step != 1
        || !selection_cards.is_empty()
        || selection_amount != i32::try_from(selection.max).unwrap_or(i32::MAX)
        || state.history.over
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let candidate_count = state.piles.get(PileId::Discard).len();
    if selection.min > 0 && candidate_count <= selection.min
        || selection.min == 0 && candidate_count == 0
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(Selector {
        pile: PileId::Discard,
        min: selection.min,
        max: selection.max,
        filter: None,
        operation: Operation::Move {
            destination: PileId::Hand,
            top: false,
        },
    })
}

fn glimmer_selector() -> Selector {
    Selector {
        pile: PileId::Hand,
        min: 1,
        max: 1,
        filter: None,
        operation: Operation::Move {
            destination: PileId::Draw,
            top: true,
        },
    }
}

fn validate_glimmer_pending(
    catalog: &Catalog,
    active: HotCard,
    next_step: u32,
    selection_amount: i32,
    has_target: bool,
    selection_cards: &[HotCard],
) -> Result<(), EngineRefusal> {
    let spec = catalog
        .spec(active.atom)
        .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
    if spec.identity.id != crate::ids::CardId::Glimmer
        || !crate::engine::admission::body_owned_select_program_is_supported(spec.row)
        || next_step != 1
        || !selection_cards.is_empty()
        || selection_amount != 0
        || has_target
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

/// Run Glimmer's synchronous FromHand/Add suffix, or request its one exact
/// external choice. The fused family body owns the preceding awaited Draw.
///
/// Verified against `Glimmer/<OnPlay>d__4::MoveNext` at RVA `0x3a173c` in the
/// installed and archived v0.111.0 DLL
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`).
pub(crate) fn execute_glimmer(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<SelectDisposition, EngineRefusal> {
    let selector = glimmer_selector();
    let candidates = candidates(state, catalog, selector)?;
    if candidates.len() <= selector.min {
        apply(state, catalog, selector, &candidates, events)?;
        Ok(SelectDisposition::Complete)
    } else if vakuu_selector_active() {
        // `VakuuCardSelector` takes the first `PutBack` Hand cards in live
        // order ([`VakuuSelectorScope`]).
        apply(
            state,
            catalog,
            selector,
            &candidates[..selector.max],
            events,
        )?;
        Ok(SelectDisposition::Complete)
    } else {
        Ok(SelectDisposition::Suspend)
    }
}

fn purity_options(
    candidates: &[HotCard],
    limit: usize,
) -> Result<Vec<Vec<HotCard>>, EngineRefusal> {
    fn append(
        candidates: &[HotCard],
        target_len: usize,
        current: &mut Vec<HotCard>,
        used: &mut [bool],
        out: &mut Vec<Vec<HotCard>>,
    ) {
        if current.len() == target_len {
            out.push(current.clone());
            return;
        }
        for index in 0..candidates.len() {
            if used[index] {
                continue;
            }
            used[index] = true;
            current.push(candidates[index]);
            append(candidates, target_len, current, used, out);
            current.pop();
            used[index] = false;
        }
    }

    let limit = limit.min(candidates.len());
    let mut out = Vec::new();
    let mut used = vec![false; candidates.len()];
    let mut current = Vec::with_capacity(limit);
    for count in 0..=limit {
        append(candidates, count, &mut current, &mut used, &mut out);
        if out.len() > u32::MAX as usize {
            return Err(EngineRefusal::CounterOverflow("Purity option count"));
        }
    }
    Ok(out)
}

pub(crate) fn special_option_count(
    state: &HotState,
    catalog: &Catalog,
    pending: &PendingSelection,
) -> Result<u32, EngineRefusal> {
    crate::engine::play::persisted_card_play_stack_is_exact(state, catalog)?;
    let (record, active) = state
        .pending_card_play(pending)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if let Err(error) = crate::engine::play::persisted_card_play_context(state, catalog, record)
        && crate::engine::play::stale_apc_pending_selector_target(state, catalog, record, active)
            .is_none()
    {
        return Err(error);
    }
    let (_, kind) = record.route()?;
    let selection_cards = record.selection_cards();
    let count = match kind {
        PendingSelectionKind::Purity => {
            let candidates = purity_candidates(state, selection_cards)?;
            purity_options(
                candidates,
                usize::try_from(record.selection_amount)
                    .map_err(|_| EngineRefusal::ContinuationNotModeled)?,
            )?
            .len()
        }
        PendingSelectionKind::SeekerStrike => seeker_candidates(state, selection_cards)?.len(),
        PendingSelectionKind::Abundance => {
            abundance_candidates(state, catalog, selection_cards)?.len()
        }
        PendingSelectionKind::Discovery => {
            discovery_candidates(state, catalog, active, selection_cards)?.len() + 1
        }
        PendingSelectionKind::Splash => {
            splash_candidates(state, catalog, active, selection_cards)?.len() + 1
        }
        PendingSelectionKind::Quasar => {
            quasar_candidates(state, catalog, active, selection_cards)?.len() + 1
        }
        PendingSelectionKind::HandCap => {
            let spec = catalog
                .spec(active.atom)
                .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
            options(
                state,
                catalog,
                hand_cap_selector(
                    state,
                    spec,
                    record.next_step,
                    record.selection_amount,
                    selection_cards,
                )?,
            )?
            .len()
        }
        PendingSelectionKind::Glimmer => {
            if state.history.over {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            validate_glimmer_pending(
                catalog,
                active,
                record.next_step,
                record.selection_amount,
                record.target.is_some(),
                selection_cards,
            )?;
            options(state, catalog, glimmer_selector())?.len()
        }
        _ => return Err(EngineRefusal::ContinuationNotModeled),
    };
    u32::try_from(count).map_err(|_| EngineRefusal::CounterOverflow("selection option count"))
}

fn apply_purity_cards(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    step_index: u32,
    selected: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    continue_selection_exhaust(
        state,
        catalog,
        crate::hot::AfterCardExhaustedReturnKind::PuritySelection,
        source_uid,
        step_index,
        PileId::Hand,
        &selected.iter().map(|card| card.uid).collect::<Vec<_>>(),
        events,
    )
}

#[allow(clippy::too_many_arguments)]
fn continue_selection_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    return_kind: crate::hot::AfterCardExhaustedReturnKind,
    source_uid: u32,
    step_index: u32,
    source_pile: PileId,
    remaining: &[u32],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    for (cursor, uid) in remaining.iter().copied().enumerate() {
        if state.history.over {
            break;
        }
        state
            .frames
            .repair_card_play_after_move(uid, source_pile, PileId::Exhaust)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let exhausted = remove_uid(state, source_pile, uid)?;
        let mut continuation = crate::hot::AfterCardExhaustedPowerRecord::for_return(return_kind);
        continuation.source_uid = Some(source_uid);
        continuation.step_index = step_index;
        continuation.flags = source_pile as u32;
        continuation.remaining = remaining[cursor + 1..].to_vec();
        let owner_live = super::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog);
        let result = super::draw::card_exhausted_with_owner(
            state,
            catalog,
            exhausted,
            continuation,
            events,
        )?;
        if owner_live || result == super::draw::CardExhaustedResult::Suspended {
            return Ok(());
        }
    }
    Ok(())
}

pub(crate) fn resume_selection_exhaust(
    state: &mut HotState,
    catalog: &Catalog,
    record: &crate::hot::AfterCardExhaustedPowerRecord,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let source_pile = PileId::ALL
        .get(record.flags as usize)
        .copied()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    continue_selection_exhaust(
        state,
        catalog,
        record.return_kind,
        record
            .source_uid
            .ok_or(EngineRefusal::ContinuationNotModeled)?,
        record.step_index,
        source_pile,
        &record.remaining,
        events,
    )
}

pub(crate) fn move_seeker_card(
    state: &mut HotState,
    uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let destination = if state.piles.get(PileId::Hand).len() >= 10 {
        PileId::Discard
    } else {
        PileId::Hand
    };
    state
        .frames
        .repair_card_play_after_move(uid, PileId::Draw, destination)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let selected = remove_uid(state, PileId::Draw, uid)?;
    state.piles.get_mut(destination).make_mut().push(selected);
    events.push(Event::CardResolved {
        uid: selected.uid,
        pile: destination,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_special_option(
    state: &mut HotState,
    catalog: &Catalog,
    kind: PendingSelectionKind,
    active: HotCard,
    record: &crate::hot::CardPlayRecord,
    selection_cards: &[HotCard],
    option_index: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    match kind {
        PendingSelectionKind::Purity => {
            let candidates = purity_candidates(state, selection_cards)?;
            let selected = purity_options(
                candidates,
                usize::try_from(record.selection_amount)
                    .map_err(|_| EngineRefusal::ContinuationNotModeled)?,
            )?
            .get(option_index as usize)
            .cloned()
            .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            // Python preflights the serial exhaust loop before clearing the
            // pending choice. Keep the direct helper equally atomic; the
            // public action clone remains the outer transaction boundary.
            let mut probe = state.clone();
            apply_purity_cards(
                &mut probe,
                catalog,
                active.uid,
                record.next_step,
                &selected,
                &mut Vec::new(),
            )?;
            apply_purity_cards(
                state,
                catalog,
                active.uid,
                record.next_step,
                &selected,
                events,
            )
        }
        PendingSelectionKind::SeekerStrike => {
            let selected = *seeker_candidates(state, selection_cards)?
                .get(option_index as usize)
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            move_seeker_card(state, selected.uid, events)
        }
        PendingSelectionKind::Abundance => {
            let selected = *abundance_candidates(state, catalog, selection_cards)?
                .get(option_index as usize)
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            // Python validates the pending screen and chosen option, then its
            // generated-card transaction is a no-op once combat is over.
            // Gate before the local insertion preflight too: allocator
            // exhaustion cannot refuse a branch that mints no card.
            if state.history.over {
                return Ok(());
            }
            let identity = catalog
                .spec(selected.atom)
                .ok_or(EngineRefusal::UnknownAtom(selected.atom))?
                .identity;
            let mut probe = state.clone();
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                &mut probe,
                catalog,
                identity,
                PileId::Hand,
                &mut Vec::new(),
            )?;
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                state,
                catalog,
                identity,
                PileId::Hand,
                events,
            )
        }
        PendingSelectionKind::Discovery => {
            let candidates = discovery_candidates(state, catalog, active, selection_cards)?;
            let index = usize::try_from(option_index)
                .map_err(|_| EngineRefusal::MalformedArgs("selection option index"))?;
            if index == candidates.len() {
                // Native `canSkip=true` returns null and bypasses the entire
                // free/generated suffix after the shuffle has been consumed.
                return Ok(());
            }
            let selected = *candidates
                .get(index)
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            // The full shuffle and screen are already committed. Native then
            // clears the modal and calls the singular generated-card command;
            // its leading ending gate suppresses allocation/history/insertion.
            if state.history.over {
                return Ok(());
            }
            let identity = catalog
                .spec(selected.atom)
                .ok_or(EngineRefusal::UnknownAtom(selected.atom))?
                .identity;
            let mut probe = state.clone();
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                &mut probe,
                catalog,
                identity,
                PileId::Hand,
                &mut Vec::new(),
            )?;
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                state,
                catalog,
                identity,
                PileId::Hand,
                events,
            )
        }
        PendingSelectionKind::Splash => {
            let candidates = splash_candidates(state, catalog, active, selection_cards)?;
            let index = usize::try_from(option_index)
                .map_err(|_| EngineRefusal::MalformedArgs("selection option index"))?;
            if index == candidates.len() {
                return Ok(());
            }
            let selected = *candidates
                .get(index)
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            if state.history.over {
                return Ok(());
            }
            let identity = catalog
                .spec(selected.atom)
                .ok_or(EngineRefusal::UnknownAtom(selected.atom))?
                .identity;
            let mut probe = state.clone();
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                &mut probe,
                catalog,
                identity,
                PileId::Hand,
                &mut Vec::new(),
            )?;
            crate::engine::cards::inject_generated_free_this_turn_bottom(
                state,
                catalog,
                identity,
                PileId::Hand,
                events,
            )
        }
        PendingSelectionKind::Quasar => {
            let candidates = quasar_candidates(state, catalog, active, selection_cards)?;
            let index = usize::try_from(option_index)
                .map_err(|_| EngineRefusal::MalformedArgs("selection option index"))?;
            if index == candidates.len() {
                return Ok(());
            }
            let selected = *candidates
                .get(index)
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            if state.history.over {
                return Ok(());
            }
            let identity = catalog
                .spec(selected.atom)
                .ok_or(EngineRefusal::UnknownAtom(selected.atom))?
                .identity;
            let mut probe = state.clone();
            crate::engine::cards::inject_generated_exact_bottom(
                &mut probe,
                catalog,
                identity,
                1,
                PileId::Hand,
                &mut Vec::new(),
            )?;
            crate::engine::cards::inject_generated_exact_bottom(
                state,
                catalog,
                identity,
                1,
                PileId::Hand,
                events,
            )
        }
        PendingSelectionKind::HandCap => {
            let spec = catalog
                .spec(active.atom)
                .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
            let selector = hand_cap_selector(
                state,
                spec,
                record.next_step,
                record.selection_amount,
                selection_cards,
            )?;
            let answer = options(state, catalog, selector)?
                .get(option_index as usize)
                .cloned()
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            apply(state, catalog, selector, &answer, events)
        }
        PendingSelectionKind::Glimmer => {
            validate_glimmer_pending(
                catalog,
                active,
                record.next_step,
                record.selection_amount,
                record.target.is_some(),
                selection_cards,
            )?;
            let answer = options(state, catalog, glimmer_selector())?
                .get(option_index as usize)
                .cloned()
                .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
            apply(state, catalog, glimmer_selector(), &answer, events)
        }
        _ => Err(EngineRefusal::ContinuationNotModeled),
    }
}

pub(crate) fn card_payload_cmp(
    state: &HotState,
    catalog: &Catalog,
    left: HotCard,
    right: HotCard,
) -> Result<Ordering, EngineRefusal> {
    card_payload_cmp_in(state, catalog, left, right, PayloadOrder::PythonTuple)
}

fn card_payload_cmp_in(
    state: &HotState,
    catalog: &Catalog,
    left: HotCard,
    right: HotCard,
    order: PayloadOrder,
) -> Result<Ordering, EngineRefusal> {
    let left_identity = catalog.spec(left.atom).expect("candidate atom").identity;
    let right_identity = catalog.spec(right.atom).expect("candidate atom").identity;
    let prefix = left_identity
        .id
        .as_str()
        .cmp(right_identity.id.as_str())
        .then_with(|| left_identity.upgrade.cmp(&right_identity.upgrade));
    if prefix != Ordering::Equal {
        return Ok(prefix);
    }

    let left_state = state.card_states.get(left.uid);
    let right_state = state.card_states.get(right.uid);
    let left_len = payload_len(left, left_identity, &left_state);
    let right_len = payload_len(right, right_identity, &right_state);

    if left_len.min(right_len) == 2 {
        return Ok(left_len.cmp(&right_len));
    }

    let ordering = optional_tuple_cmp(
        logical_enchantment_tuple(left_identity, &left_state),
        logical_enchantment_tuple(right_identity, &right_state),
        "selection enchantment payload order",
        order,
    )?;
    if ordering != Ordering::Equal || left_len.min(right_len) == 3 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }

    // Slots 3 and 4 are ordered local/transient keyword tuples. Permanent
    // Retain/Sly tuples compare in Python's exact sorted string order.
    let ordering = left_state
        .local_keyword_rank()
        .cmp(&right_state.local_keyword_rank());
    if ordering != Ordering::Equal || left_len.min(right_len) == 4 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }
    let ordering = left_state
        .transient_keyword_rank()
        .cmp(&right_state.transient_keyword_rank());
    if ordering != Ordering::Equal || left_len.min(right_len) == 5 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }

    // Slot 5 is the remaining modeled value before the always-None
    // sovereign slot 6.
    let ordering = optional_tuple_cmp(
        visible_enchantment_state(left_identity, &left_state).map(|amount| {
            (
                left_identity.enchantment.expect("state owner").id.as_str(),
                amount,
            )
        }),
        visible_enchantment_state(right_identity, &right_state).map(|amount| {
            (
                right_identity.enchantment.expect("state owner").id.as_str(),
                amount,
            )
        }),
        "selection enchantment-state payload order",
        order,
    )?;
    if ordering != Ordering::Equal || left_len.min(right_len) == 6 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }

    // Slot 6 is Sovereign Blade's mutable `(tag, damage, repeats)` tuple and
    // precedes slot 7 even when the latter is absent.
    let ordering = optional_tuple_cmp(
        (left.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE != 0).then_some((
            "SOVEREIGN_BLADE",
            left_state.damage_growth,
            1,
        )),
        (right.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE != 0).then_some((
            "SOVEREIGN_BLADE",
            right_state.damage_growth,
            1,
        )),
        "selection Sovereign Blade payload order",
        order,
    )?;
    if ordering != Ordering::Equal || left_len.min(right_len) == 7 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }

    let ordering = physical_state_cmp(&left_state, &right_state);
    if ordering != Ordering::Equal || left_len.min(right_len) == 8 {
        return Ok(ordering.then_with(|| left_len.cmp(&right_len)));
    }

    // Slot 8 is the first opaque tail row. Ringing-only therefore compares
    // its CARD_AFFLICTION tag against GA's GENETIC_ALGORITHM_STATE tag;
    // GA+Ringing carries the fixed Ringing row only after the GA row.
    let ordering = card_tail_cmp(left, &left_state, right, &right_state)?;
    Ok(ordering.then_with(|| left_len.cmp(&right_len)))
}

fn logical_enchantment_tuple(
    identity: crate::catalog::CardIdentity,
    state: &CardInstanceState,
) -> Option<(&'static str, i32)> {
    identity.enchantment.map(|row| {
        (
            if row.id == crate::ids::EnchantmentId::Glam && state.enchantment_state.get() == Some(1)
            {
                "GLAM_USED"
            } else {
                row.id.as_str()
            },
            row.amount,
        )
    })
}

fn visible_enchantment_state(
    identity: crate::catalog::CardIdentity,
    state: &CardInstanceState,
) -> Option<i32> {
    state.enchantment_state.get().filter(|amount| {
        !identity
            .enchantment
            .is_some_and(|row| row.id == crate::ids::EnchantmentId::Glam && *amount == 1)
    })
}

fn payload_len(
    card: HotCard,
    identity: crate::catalog::CardIdentity,
    state: &CardInstanceState,
) -> usize {
    let genetic_algorithm = card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0;
    let affliction = card.flags & (CARD_FLAG_BOUND | CARD_FLAG_HEXED | CARD_FLAG_RINGING) != 0;
    if genetic_algorithm && affliction {
        10
    } else if genetic_algorithm || affliction {
        9
    } else if card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0 {
        8
    } else if card.flags & CARD_FLAG_SOVEREIGN_BLADE_STATE != 0 {
        7
    } else if visible_enchantment_state(identity, state).is_some() {
        6
    } else if state.transient_retain || state.transient_sly() {
        5
    } else if state.local_retain || state.local_sly {
        4
    } else if identity.enchantment.is_some() {
        3
    } else {
        2
    }
}

fn card_tail_cmp(
    left_card: HotCard,
    left_state: &CardInstanceState,
    right_card: HotCard,
    right_state: &CardInstanceState,
) -> Result<Ordering, EngineRefusal> {
    fn affliction(card: HotCard) -> Option<(&'static str, &'static str, i32)> {
        if card.flags & CARD_FLAG_BOUND != 0 {
            Some(("CARD_AFFLICTION", "BOUND", 3))
        } else if card.flags & CARD_FLAG_HEXED != 0 {
            Some(("CARD_AFFLICTION", "HEXED", 2))
        } else if card.flags & CARD_FLAG_RINGING != 0 {
            Some(("CARD_AFFLICTION", "RINGING", 1))
        } else {
            None
        }
    }
    let left_genetic = left_card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0;
    let right_genetic = right_card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0;
    match (left_genetic, right_genetic) {
        (false, false) => Ok(affliction(left_card).cmp(&affliction(right_card))),
        (false, true) => Ok("CARD_AFFLICTION".cmp("GENETIC_ALGORITHM_STATE")),
        (true, false) => Ok("GENETIC_ALGORITHM_STATE".cmp("CARD_AFFLICTION")),
        (true, true) => {
            let left = left_state.genetic_algorithm;
            let right = right_state.genetic_algorithm;
            let growth = left.growth().cmp(&right.growth());
            if growth != Ordering::Equal {
                return Ok(growth);
            }
            match (left.deck_row(), right.deck_row()) {
                (Some(left), Some(right)) => Ok(left
                    .cmp(&right)
                    .then_with(|| affliction(left_card).cmp(&affliction(right_card)))),
                (None, None) => Ok(affliction(left_card).cmp(&affliction(right_card))),
                // Python reaches the third GA row field only after equal
                // growth, then refuses to order None against an integer.
                _ => Err(EngineRefusal::MalformedArgs(
                    "selection Genetic Algorithm deck-row order",
                )),
            }
        }
    }
}

fn optional_tuple_cmp<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    refusal: &'static str,
    order: PayloadOrder,
) -> Result<Ordering, EngineRefusal> {
    match (left, right, order) {
        (Some(left), Some(right), _) => Ok(left.cmp(&right)),
        (None, None, _) => Ok(Ordering::Equal),
        // Python 3 does not order None against a tuple. Mirror that refusal
        // wherever the candidate order can reach game state.
        (_, _, PayloadOrder::PythonTuple) => Err(EngineRefusal::MalformedArgs(refusal)),
        // #2985: where the order only labels the answers, native has no
        // order to match (see `candidate_order_is_label_only`). `None`
        // first agrees with the payload-length shortcut above, which already
        // puts a bare card before its enchanted twin.
        (None, Some(_), PayloadOrder::LabelOnly) => Ok(Ordering::Less),
        (Some(_), None, PayloadOrder::LabelOnly) => Ok(Ordering::Greater),
    }
}

fn physical_state_cmp(left: &CardInstanceState, right: &CardInstanceState) -> Ordering {
    fn star_rows_cmp(left: &CardInstanceState, right: &CardInstanceState) -> Ordering {
        // Compare the complete canonical Star list. Touch of Insanity can
        // append combat rows after temporary rows, including repeated writes.
        // The admitted expiration tags preserve canonical (0,false,false)
        // before (0,true,true), with ordinary lexicographic prefix ordering.
        left.local_cost_modifiers
            .star_cost_expirations(left.free_star_cost_this_turn_or_played_rows)
            .map(|expiration| expiration as u8)
            .cmp(
                right
                    .local_cost_modifiers
                    .star_cost_expirations(right.free_star_cost_this_turn_or_played_rows)
                    .map(|expiration| expiration as u8),
            )
    }
    let left_rows = left.local_cost_modifiers.as_slice();
    let right_rows = right.local_cost_modifiers.as_slice();
    for (left, right) in left_rows.iter().zip(right_rows) {
        let ordering = (left.kind as u8)
            .cmp(&(right.kind as u8))
            .then_with(|| left.amount.cmp(&right.amount))
            .then_with(|| (left.expiration as u8).cmp(&(right.expiration as u8)))
            .then_with(|| left.reduce_only.cmp(&right.reduce_only));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left_rows
        .len()
        .cmp(&right_rows.len())
        .then_with(|| {
            left.exact_damage_growth()
                .expect("valid card state has one damage-growth representation")
                .cmp(
                    &right
                        .exact_damage_growth()
                        .expect("valid card state has one damage-growth representation"),
                )
        })
        // The legacy boolean is always False in the canonical inverse.
        .then_with(|| star_rows_cmp(left, right))
        // Python compares the optional sixth tuple member by tuple length
        // first, then by its nonnegative integer value when both are present.
        .then_with(
            || match (left.base_replay_count(), right.base_replay_count()) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(left), Some(right)) => left.cmp(&right),
            },
        )
}

fn options(
    state: &HotState,
    catalog: &Catalog,
    selector: Selector,
) -> Result<Vec<Vec<HotCard>>, EngineRefusal> {
    let candidates = sorted_exact_candidates(state, catalog, selector)?;
    let lo = selector.min.min(candidates.len());
    let hi = selector.max.min(candidates.len());
    let mut options = vec![Vec::new()];
    for card in candidates {
        let mut next = Vec::with_capacity(options.len().saturating_mul(2));
        for prior in options {
            next.push(prior.clone());
            if prior.len() < hi {
                let mut with = prior;
                with.push(card);
                next.push(with);
            }
        }
        options = next;
    }
    options.retain(|answer| (lo..=hi).contains(&answer.len()));
    if selection_order_is_observable(selector.operation) {
        options = pick_order_variants(options);
    }
    if options.len() > u32::MAX as usize {
        return Err(EngineRefusal::CounterOverflow("selection option count"));
    }
    Ok(options)
}

/// Whether this consumer operation makes SELECTION ORDER observable.
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `CardSelectCmd/<FromCombatPile>d__20::MoveNext` RVA `0x3e5e84` publishes the
/// local selector's `GetSelectedCards` list through
/// `PlayerChoiceResult::FromMutableCombatCards` + `SyncLocalChoice`
/// (IL_0433/IL_0438) and then *returns* the synchronized net answer read back
/// by `PlayerChoiceResult::AsCombatCards` (IL_04c3) on every peer, the local
/// one included. `AsCombatCards` RVA `0x10d350` returns the `_combatCards`
/// field verbatim — there is no `OrderBy` or `Sort` on that path, and none in
/// `CardPileCmd`, whose plural `Add` inserts each card in list order. So a
/// `Move` sink writes the player's own pick order straight into the
/// destination pile: two orders of one payload-uniform pick are two different
/// legal actions, and both have to be enumerated (#2521).
///
/// The Discard and Exhaust sinks consume that list serially too (#2524):
///
/// * `Discard` (`SelectOp::Discard` and `SelectOp::DiscardAll`) is plural
///   `CardCmd::Discard`. `HiddenDaggers/<OnPlay>d__6::MoveNext` RVA
///   `0x3a5480` awaits `FromHandForDiscard` at IL_0057 and hands the same
///   enumerable to `CardCmd::Discard` at IL_00c0;
///   `Prepared/<OnPlay>d__3::MoveNext` RVA `0x3b3678` does the same at
///   IL_00d3/IL_013c. `FromHandForDiscard` d__29 (RVA `0x3e7a64`) returns
///   `FromHand` d__28 (RVA `0x3e7568`) verbatim, which reads the answer back
///   through `AsCombatCards` at IL_0400. `CardCmd/<Discard>d__3::MoveNext`
///   RVA `0x3e01ac` delegates to `DiscardAndDraw(cards, 0)` at IL_0023, and
///   `CardCmd/<DiscardAndDraw>d__4::MoveNext` RVA `0x3e0274` `ToList`s the
///   input (IL_0041) and, per element in list order, captures a Sly child
///   (IL_00f6), `CardPileCmd::Add`s it at Discard/Bottom (IL_011d), records
///   `CardDiscarded` (IL_018e) and awaits `Hook::AfterCardDiscarded`
///   (IL_01a5), then auto-plays the Sly children in that same order
///   (IL_02fd). Discard pile order, the per-card hook order and Sly child
///   order all follow pick order.
/// * `Exhaust` walks the answer one `CardCmd::Exhaust` at a time in list
///   order ([`continue_selection_exhaust`]), so Exhaust pile order and the
///   per-card `AfterCardExhausted` order follow pick order.
///
/// Over the current card catalog the only multi-pick programs on these sinks
/// are Hidden Daggers L0/L1 and Prepared L1 (`2..2`, two orders per pair);
/// every other discard/exhaust select is exact-one, where the fan-out is the
/// identity. The two potion sinks with the same consumers (Ashwater, Gambler's
/// Brew) enumerate outside this selector, in `engine/potions.rs`.
///
/// The remaining operations are exact-one at every admitted call site, so the
/// sub-multiset and its orders coincide; the per-sink verdict is recorded in
/// `solver/invariant-walks/2026-09-16-select-pick-order.md` and, for the
/// discard/exhaust family, `solver/invariant-walks/2026-09-25-issue2524-discard-exhaust-pick-order.md`.
fn selection_order_is_observable(operation: Operation) -> bool {
    match operation {
        Operation::Move { .. }
        | Operation::RegentHandTransform { .. }
        | Operation::ChargeTransform { .. }
        | Operation::Discard
        | Operation::Exhaust => true,
        Operation::Upgrade
        | Operation::ApplyPermanentRetain
        | Operation::ApplyPermanentEthereal
        | Operation::ApplySingleTurnSly
        | Operation::Decisions
        | Operation::CloneGenerated { .. }
        | Operation::SeanceTransform
        | Operation::Transfigure => false,
    }
}

/// Fan each selected sub-multiset out into its distinct pick ORDERS.
///
/// Sub-multiset order is preserved, so the enumeration stays subset-major and
/// each subset's permutations follow it in index-lexicographic order — exactly
/// what Python's `_pick_order_variants` emits from
/// `itertools.permutations`. The two lists must agree element for element: the
/// differential compares the ordered legal-action list, and the wire's
/// `option_index` is positional.
///
/// No deduplication is needed here. Every subset is built from
/// `sorted_exact_candidates`, whose entries are distinct live cards with
/// distinct uids, so a subset's permutations are all distinct. (Python
/// deduplicates because its legacy payload mode — the untagged Foregone
/// turn-start surface, which this crate does not model — can hold
/// equal-payload values.)
fn pick_order_variants(subsets: Vec<Vec<HotCard>>) -> Vec<Vec<HotCard>> {
    fn permute(
        subset: &[HotCard],
        used: &mut [bool],
        current: &mut Vec<HotCard>,
        out: &mut Vec<Vec<HotCard>>,
    ) {
        if current.len() == subset.len() {
            out.push(current.clone());
            return;
        }
        for index in 0..subset.len() {
            if used[index] {
                continue;
            }
            used[index] = true;
            current.push(subset[index]);
            permute(subset, used, current, out);
            current.pop();
            used[index] = false;
        }
    }

    let mut out = Vec::with_capacity(subsets.len());
    for subset in subsets {
        if subset.len() < 2 {
            out.push(subset);
            continue;
        }
        let mut used = vec![false; subset.len()];
        let mut current = Vec::with_capacity(subset.len());
        permute(&subset, &mut used, &mut current, &mut out);
    }
    out
}

/// Run the synchronous branch of a generated select, or request suspension.
pub(crate) fn execute(ctx: &mut StepCtx<'_>) -> Result<SelectDisposition, EngineRefusal> {
    let selector = selector(ctx)?;
    let candidates = candidates(ctx.state, ctx.catalog, selector)?;
    if candidates.len() <= selector.min {
        // A native `CardSelectCmd` signals the context before this auto-take
        // (`<FromHand>d__28` RVA `0x3e7568` IL_00b3 before IL_0152-018d;
        // `<FromCombatPile>d__20` RVA `0x3e5e84` IL_00ae before
        // IL_0140-017c) unless the combat is ending (IL_0036) or a
        // `Selector` is set (#3387, `hook_action::AUTO_RESOLVED_CHOICE`).
        super::hook_action::note_unprompted_select(
            !ctx.state.history.over && !vakuu_selector_active(),
        );
        apply(ctx.state, ctx.catalog, selector, &candidates, ctx.events)?;
        Ok(SelectDisposition::Complete)
    } else if vakuu_selector_active() && vakuu_program_is_exact(ctx.spec, selector) {
        // `VakuuCardSelector` takes the first `MaxSelect` options in live
        // pile order ([`VakuuSelectorScope`]).
        apply(
            ctx.state,
            ctx.catalog,
            selector,
            &candidates[..selector.max],
            ctx.events,
        )?;
        Ok(SelectDisposition::Complete)
    } else {
        // Python normalizes a clone to enumerate immutable physical picks,
        // but does not promote the source State's exact-pile projection just
        // because a frame selection is pending. Rust already carries stable
        // internal UIDs, and the compact answer ordinal keeps them off the
        // action wire, so suspension itself is projection-neutral.
        Ok(SelectDisposition::Suspend)
    }
}

pub(crate) fn option_count(
    state: &HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    step_index: usize,
) -> Result<u32, EngineRefusal> {
    let step = *catalog
        .steps(spec)
        .get(step_index)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if step.kind != StepKind::Select {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let count = options(
        state,
        catalog,
        selector_from_args(catalog, spec, catalog.args(step.args))?,
    )?
    .len();
    Ok(count as u32)
}

/// Read-only replay metadata: the answer that applies exactly `uids`, in that
/// order, for a pending selection whose ordered answers are accepted but not
/// all offered by `legal_actions` — today Ashwater's and Gambler's Brew's
/// ordered extension (#2524, `potions::potion_selection_at`). `None` means
/// this pending selection has no such extension (resolve it against the
/// legal list) or `uids` names no accepted answer.
pub fn ordered_selection_answer(
    state: &HotState,
    catalog: &Catalog,
    uids: &[u32],
) -> Result<Option<super::SelectionAnswer>, EngineRefusal> {
    Ok(
        super::potions::ordered_selection_ordinal(state, catalog, uids)?
            .map(super::SelectionAnswer::OptionIndex),
    )
}

/// Read-only replay metadata: the unique `select` answer that applies exactly
/// the recorded physical `uids`, in that order (#3125).
///
/// The ordered-extension answers of [`ordered_selection_answer`] come first
/// (they are accepted but not offered). Otherwise every `select` that
/// [`super::legal_actions`] offers is decoded through [`selected_card_uids`]
/// — the transition's own option enumerator — and the answers naming exactly
/// `uids` that the engine also applies are kept. This is the replay
/// resolver's per-candidate rule (`rust_replay._selection`, eval_suite
/// `_multi_card_selection`) run in-process, so an ordered pick surface such as
/// Gambling Chip's `Σ_k P(n, k)` (326 answers for a 5-card hand) or a
/// 3-of-8 frame pick (336) resolves without one wire round trip per answer.
///
/// `None`: no offered answer names `uids`. Two or more answers naming them
/// refuse by name rather than pick one. No new native semantics: decoding
/// and applying are the existing transitions, unchanged.
pub fn recorded_selection_answer(
    state: &HotState,
    catalog: &Catalog,
    uids: &[u32],
) -> Result<Option<super::SelectionAnswer>, EngineRefusal> {
    if let Some(answer) = ordered_selection_answer(state, catalog, uids)? {
        return Ok(Some(answer));
    }
    let offered =
        super::legal_actions(state, catalog)
            .into_iter()
            .filter_map(|action| match action {
                super::Action::Select { answer } => Some(answer),
                _ => None,
            });
    unique_recorded_answer(
        offered,
        uids,
        |answer| selected_card_uids(state, catalog, answer),
        |answer| super::apply_action(state, catalog, &super::Action::Select { answer }).is_ok(),
    )
}

/// The matching rule behind [`recorded_selection_answer`], over an explicit
/// candidate list: a candidate whose decode refuses, is nonphysical (`None`)
/// or names other cards is skipped; a match the engine refuses to apply is
/// skipped; a second surviving match refuses.
fn unique_recorded_answer(
    offered: impl IntoIterator<Item = super::SelectionAnswer>,
    uids: &[u32],
    decode: impl Fn(super::SelectionAnswer) -> Result<Option<Vec<u32>>, EngineRefusal>,
    applies: impl Fn(super::SelectionAnswer) -> bool,
) -> Result<Option<super::SelectionAnswer>, EngineRefusal> {
    let mut found = None;
    for answer in offered {
        if !matches!(decode(answer), Ok(Some(selected)) if selected == uids) {
            continue;
        }
        if !applies(answer) {
            continue;
        }
        if found.replace(answer).is_some() {
            return Err(EngineRefusal::MalformedArgs(
                "recorded selection names more than one answer",
            ));
        }
    }
    Ok(found)
}

/// Read-only review metadata. Decode physical choices through the same option
/// enumerators used by the transition; never infer an ordinal from pile order
/// or from cards that happen to move during downstream listeners.
/// None means a non-physical choice (generated cards or an enemy dialog).
pub fn selected_card_uids(
    state: &HotState,
    catalog: &Catalog,
    answer: super::SelectionAnswer,
) -> Result<Option<Vec<u32>>, EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if let super::SelectionAnswer::CardUid(uid) = answer {
        return Ok(Some(vec![uid]));
    }
    let super::SelectionAnswer::OptionIndex(ordinal) = answer else {
        unreachable!()
    };
    let index = ordinal as usize;
    let bad = || EngineRefusal::MalformedArgs("selection option index");
    let cards = if pending.is_relic_selection() {
        let relic = state.fanouts.batch_nine_relic_pending().ok_or_else(bad)?;
        match relic.kind {
            crate::hot::RelicPendingKind::GamblingChip => {
                super::relics::gambling_selection(&relic.entries, ordinal).ok_or_else(bad)?
            }
            crate::hot::RelicPendingKind::ToastyMittens => {
                vec![relic.entries.get(index).ok_or_else(bad)?.card]
            }
            _ => return Ok(None),
        }
    } else if pending.stratagem_potion_record(&state.frames).is_some()
        || pending.stratagem_draw_record(&state.frames).is_some()
    {
        super::draw::stratagem_selection_at(state, catalog, ordinal)?
    } else if let Some(record) = pending.potion_finish_record(&state.frames) {
        let candidates = record.candidates().collect::<Vec<_>>();
        match record.name {
            crate::ids::PotionId::Ashwater | crate::ids::PotionId::GamblersBrew => {
                super::potions::potion_selection_at(
                    state,
                    catalog,
                    record.name,
                    &candidates,
                    ordinal,
                )?
            }
            _ => vec![*candidates.get(index).ok_or_else(bad)?],
        }
    } else if let Some(record) = pending.turn_start_hand_choice_record(&state.frames) {
        super::turn::hand_choice_selection(
            &record.entries().collect::<Vec<_>>(),
            record.amount,
            ordinal,
        )?
        .into_iter()
        .map(|entry| entry.card)
        .collect()
    } else if let Some(record) = pending.foregone_before_hand_draw_record(&state.frames) {
        super::turn::foregone_selected_cards(&record.to_owned(), ordinal)?
            .into_iter()
            .map(|entry| entry.card)
            .collect()
    } else if pending.generation_potion_record(&state.frames).is_some()
        || pending.enemy_phase_record(&state.frames).is_some()
    {
        return Ok(None);
    } else {
        let (record, active) = state.pending_card_play(pending).ok_or_else(bad)?;
        let spec = catalog
            .spec(active.atom)
            .ok_or(EngineRefusal::UnknownAtom(active.atom))?;
        let (_, kind) = record.route()?;
        let frozen = record.selection_cards();
        match kind {
            PendingSelectionKind::Program => {
                let step = catalog
                    .steps(spec)
                    .get(record.next_step.checked_sub(1).ok_or_else(bad)? as usize)
                    .ok_or_else(bad)?;
                let selector = selector_from_args(catalog, spec, catalog.args(step.args))?;
                options(state, catalog, selector)?
                    .get(index)
                    .cloned()
                    .ok_or_else(bad)?
            }
            PendingSelectionKind::Purity => purity_options(
                purity_candidates(state, frozen)?,
                record.selection_amount as usize,
            )?
            .get(index)
            .cloned()
            .ok_or_else(bad)?,
            PendingSelectionKind::SeekerStrike => vec![
                *seeker_candidates(state, frozen)?
                    .get(index)
                    .ok_or_else(bad)?,
            ],
            PendingSelectionKind::HandCap => options(
                state,
                catalog,
                hand_cap_selector(
                    state,
                    spec,
                    record.next_step,
                    record.selection_amount,
                    frozen,
                )?,
            )?
            .get(index)
            .cloned()
            .ok_or_else(bad)?,
            PendingSelectionKind::Glimmer => options(state, catalog, glimmer_selector())?
                .get(index)
                .cloned()
                .ok_or_else(bad)?,
            PendingSelectionKind::Abundance
            | PendingSelectionKind::Discovery
            | PendingSelectionKind::Splash
            | PendingSelectionKind::Quasar => return Ok(None),
            PendingSelectionKind::Replay | PendingSelectionKind::Tutor => return Err(bad()),
        }
    };
    Ok(Some(cards.iter().map(|card| card.uid).collect()))
}

pub(crate) fn apply_option(
    state: &mut HotState,
    catalog: &Catalog,
    spec: &CardSpec,
    step_index: usize,
    option_index: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let step = *catalog
        .steps(spec)
        .get(step_index)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if step.kind != StepKind::Select {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let selector = selector_from_args(catalog, spec, catalog.args(step.args))?;
    let answer = options(state, catalog, selector)?
        .get(option_index as usize)
        .cloned()
        .ok_or(EngineRefusal::MalformedArgs("selection option index"))?;
    apply(state, catalog, selector, &answer, events)
}

/// The one live card with `uid` in `pile`, without moving it.
fn unique_uid(state: &HotState, pile: PileId, uid: u32) -> Result<HotCard, EngineRefusal> {
    let mut matches = state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .filter(|card| card.uid == uid);
    match (matches.next(), matches.next()) {
        (Some(card), None) => Ok(*card),
        _ => Err(EngineRefusal::FrozenCardVanished { uid, pile }),
    }
}

fn remove_uid(state: &mut HotState, pile: PileId, uid: u32) -> Result<HotCard, EngineRefusal> {
    let cards = state.piles.get_mut(pile).make_mut();
    let matches: Vec<_> = cards
        .iter()
        .enumerate()
        .filter_map(|(index, card)| (card.uid == uid).then_some(index))
        .collect();
    if matches.len() != 1 {
        return Err(EngineRefusal::FrozenCardVanished { uid, pile });
    }
    Ok(cards.remove(matches[0]))
}

/// Transfigure's writer over its one selected Hand card (#3291).
///
/// Current v0.111.0 `sts2.dll`
/// (`sha256:9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`),
/// `Transfigure/<OnPlay>d__9::MoveNext` RVA `0x3c4438`: the awaited answer is
/// stored whole (IL_009b), the `TriggerAnim('Cast')` wait (IL_00c0) is
/// presentation, and the body then walks the answer (IL_011e-IL_017b). Per
/// selected card, in order:
///
/// 1. Cost (IL_0135-IL_015c). `EnergyCost.CostsX` true (IL_013c) skips to
///    step 2, as does `EnergyCost.GetWithModifiers(0)` below zero
///    (IL_0145-IL_0151). With `CostModifiers.None`,
///    `CardEnergyCost::GetWithModifiers` RVA `0x11e044` returns `_base`
///    unchanged when `_base < 0` (IL_0022-IL_002c) and otherwise skips both the
///    local (flag 2, IL_0037) and global (flag 4, IL_0081) walks and returns
///    `Max(0, _base)` (IL_00c3-IL_00c5), so the skip is exactly
///    `CostsX || _base < 0`. `_base` is the level's printed cost here: its one
///    writer `SetCustomBaseCost` is called only from `UpgradeBy`, which the
///    engine models as the next level's row. Every other card receives
///    `EnergyCost.AddThisCombat(1, false)` (IL_0153-IL_015c);
///    `CardEnergyCost::AddThisCombat` RVA `0x11e28d` appends
///    `LocalCostModifier(1, 2 = Add, 0 = ThisCombat, false)` (IL_0005-IL_0014;
///    `LocalCostModifier::.ctor` RVA `0x11f1fa` takes Amount, Type, Expiration,
///    IsReduceOnly in that order).
/// 2. Replay (IL_0161-IL_016f): `BaseReplayCount = BaseReplayCount + 1`,
///    unconditionally, including a skipped-cost card.
///
/// The card is not moved: its pile position, uid and every other payload
/// field are kept. Both writes are per-instance state (the #2502 class) and
/// ride the existing slot-6 replay count and slot-7 ordered cost rows, so the
/// instance is marked [`CARD_FLAG_DEFAULT_PHYSICAL_STATE`] and exact piles
/// are published, as for Hidden Gem and Enlightenment. A uid-less legacy
/// card has no physical identity to carry the rows and refuses by name.
fn apply_transfigure(
    state: &mut HotState,
    catalog: &Catalog,
    selector: Selector,
    answer: &[HotCard],
) -> Result<(), EngineRefusal> {
    let selected = match answer {
        [] => return Ok(()),
        [selected] => *selected,
        _ => {
            return Err(EngineRefusal::MalformedArgs(
                "Transfigure exact-one selection",
            ));
        }
    };
    if selector.pile != PileId::Hand || selected.flags & crate::hot::CARD_FLAG_LEGACY != 0 {
        return Err(EngineRefusal::MalformedArgs(
            "Transfigure selected physical identity",
        ));
    }
    let matches: Vec<_> = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(_, live)| snapshot_matches_live(**live, selected))
        .map(|(index, _)| index)
        .collect();
    let [index] = matches[..] else {
        return Err(EngineRefusal::FrozenCardVanished {
            uid: selected.uid,
            pile: PileId::Hand,
        });
    };
    let spec = catalog
        .spec(selected.atom)
        .ok_or(EngineRefusal::UnknownAtom(selected.atom))?;
    let mut instance = state.card_states.get(selected.uid);
    let replay = instance
        .base_replay_count()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("BaseReplayCount"))?;
    instance
        .set_base_replay_count(Some(replay))
        .ok_or(EngineRefusal::CounterOverflow("BaseReplayCount"))?;
    state.card_states.set(selected.uid, instance);
    if !(spec.x_cost || spec.cost < 0) {
        state.card_states.append_local_cost_modifier(
            selected.uid,
            crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Add,
                amount: 1,
                expiration: crate::hot::LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
    }
    state.piles.get_mut(PileId::Hand).make_mut()[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    state.exact_piles = true;
    Ok(())
}

fn apply(
    state: &mut HotState,
    catalog: &Catalog,
    selector: Selector,
    answer: &[HotCard],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    match selector.operation {
        Operation::Transfigure => apply_transfigure(state, catalog, selector, answer),
        Operation::Decisions => {
            if answer.len() > 1 {
                return Err(EngineRefusal::MalformedArgs("Decisions selection"));
            }
            if let Some(card) = answer.first() {
                super::play::decisions_serial_auto_plays(state, catalog, *card, events)?;
            }
            Ok(())
        }
        Operation::SeanceTransform => {
            // Seance/<OnPlay>d__7::MoveNext (v0.111.0 RVA 0x3b8ed8) selects
            // one Draw card and calls TransformTo<Soul>. The integer 1 is
            // CardPreviewStyle, not an upgrade: TransformTo (0x3e15f0) creates
            // a fresh L0 model and delegates through singular Transform
            // (0x3e09f0) to the plural command (0x3e0ae0) with one record.
            // Thus the existing fixed-payload transaction preserves its pile
            // index, fresh identity, generated history and listener order.
            if selector.pile != PileId::Draw || answer.len() > 1 {
                return Err(EngineRefusal::MalformedArgs("Seance selection"));
            }
            let frozen: Vec<_> = answer
                .iter()
                .map(|card| (*card, state.card_states.get(card.uid)))
                .collect();
            super::cards::bulk_transform_fixed_same_pile(
                state,
                catalog,
                PileId::Draw,
                &frozen,
                crate::catalog::CardIdentity {
                    id: crate::ids::CardId::Soul,
                    upgrade: 0,
                    enchantment: None,
                },
                events,
            )?;
            Ok(())
        }
        Operation::ChargeTransform { upgrade } => {
            // v111 Charge/<OnPlay>d__5 MoveNext 0x391b28 awaits each
            // TransformTo<MinionDiveBomb> in selection order, THEN upgrades
            // the returned card. Entry/generated hooks see a fresh L0 card.
            if selector.pile != PileId::Draw {
                return Err(EngineRefusal::MalformedArgs("Charge transform pile"));
            }
            for selected in answer {
                let original = state
                    .piles
                    .get(PileId::Draw)
                    .as_slice()
                    .iter()
                    .find(|card| card.uid == selected.uid)
                    .copied()
                    .ok_or(EngineRefusal::FrozenCardVanished {
                        uid: selected.uid,
                        pile: PileId::Draw,
                    })?;
                let instance = state.card_states.get(original.uid);
                let replacements = super::cards::bulk_transform_fixed_same_pile(
                    state,
                    catalog,
                    PileId::Draw,
                    &[(original, instance)],
                    crate::catalog::CardIdentity {
                        id: crate::ids::CardId::MinionDiveBomb,
                        upgrade: 0,
                        enchantment: None,
                    },
                    events,
                )?;
                if upgrade > 0 {
                    super::cards::upgrade_live_cards_once(
                        state,
                        catalog,
                        &replacements.iter().map(|card| card.uid).collect::<Vec<_>>(),
                    )?;
                }
            }
            Ok(())
        }
        Operation::RegentHandTransform {
            replacement,
            upgrade,
        } => {
            // Begone 0x38c19c and Guards 0x3a29e8 create/upgrade a fresh
            // MinionStrike/MinionSacrifice BEFORE singular Transform. Guards
            // awaits each singular command in the player's selection order.
            if selector.pile != PileId::Hand {
                return Err(EngineRefusal::MalformedArgs("Regent hand transform pile"));
            }
            for selected in answer {
                let original = state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .find(|card| card.uid == selected.uid)
                    .copied()
                    .ok_or(EngineRefusal::FrozenCardVanished {
                        uid: selected.uid,
                        pile: PileId::Hand,
                    })?;
                let instance = state.card_states.get(original.uid);
                super::cards::bulk_transform_fixed_same_pile(
                    state,
                    catalog,
                    PileId::Hand,
                    &[(original, instance)],
                    crate::catalog::CardIdentity {
                        id: replacement,
                        upgrade,
                        enchantment: None,
                    },
                    events,
                )?;
            }
            Ok(())
        }
        Operation::Discard => {
            // The selected objects stay in their pile until DiscardAndDraw's
            // own per-card CardPileCmd.Add moves them, in pick order (#3102),
            // behind its entry and per-card ending gates (#3075).
            let mut picked = Vec::with_capacity(answer.len());
            for card in answer {
                picked.push(unique_uid(state, selector.pile, card.uid)?);
            }
            discard_and_draw(state, catalog, selector.pile, &picked, 0, events)
        }
        Operation::Exhaust => {
            let (source_uid, step_index) =
                if super::draw::ordinary_after_card_exhausted_owner_is_live(state, catalog) {
                    let crate::frame::Frame::CardPlay { record } = state
                        .frames
                        .top()
                        .ok_or(EngineRefusal::ContinuationNotModeled)?
                    else {
                        return Err(EngineRefusal::ContinuationNotModeled);
                    };
                    let owner = state
                        .frames
                        .card_play(record)
                        .ok_or(EngineRefusal::ContinuationNotModeled)?;
                    (owner.uid, owner.next_step)
                } else {
                    (0, 0)
                };
            continue_selection_exhaust(
                state,
                catalog,
                crate::hot::AfterCardExhaustedReturnKind::GenericSelection,
                source_uid,
                step_index,
                selector.pile,
                &answer.iter().map(|card| card.uid).collect::<Vec<_>>(),
                events,
            )
        }
        Operation::Upgrade => {
            let selected = match answer {
                [] => return Ok(()),
                [selected] => selected,
                _ => {
                    return Err(EngineRefusal::MalformedArgs(
                        "Armaments L0 exact-one upgrade selection",
                    ));
                }
            };
            // Corrects frozen `combat_sim._apply_select_op` (frozen Python, deleted #2827),
            // whose remove/reappend changes Hand order.
            // v0.111.0 Armaments/<OnPlay>d__5::MoveNext (RVA 0x38a310) selects from Hand
            // then calls CardCmd::Upgrade on the selected card. There is no
            // pile move: preserve its physical position, as the native turn-3
            // checkpoint in the Ovicopter prefix fixture also records.
            let mut upgraded = state.clone();
            if selector.pile != PileId::Hand
                || upgraded
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .find(|card| card.uid == selected.uid)
                    != Some(selected)
            {
                return Err(EngineRefusal::MalformedArgs(
                    "Armaments L0 selected physical identity",
                ));
            }
            crate::engine::cards::upgrade_live_cards_once(&mut upgraded, catalog, &[selected.uid])?;
            *state = upgraded;
            Ok(())
        }
        Operation::ApplyPermanentRetain
        | Operation::ApplyPermanentEthereal
        | Operation::ApplySingleTurnSly => {
            for frozen in answer {
                let matches: Vec<_> = state
                    .piles
                    .get(selector.pile)
                    .as_slice()
                    .iter()
                    .filter(|live| snapshot_matches_live(**live, *frozen))
                    .collect();
                if matches.len() != 1 {
                    return Err(EngineRefusal::FrozenCardVanished {
                        uid: frozen.uid,
                        pile: selector.pile,
                    });
                }
            }
            // Python commits every selected physical-card keyword rewrite
            // with `force_exact=True`. The promotion is unconditional for a
            // nonempty answer because CompareTo ignores keyword slots, even
            // when no equal sibling is currently present.
            if !answer.is_empty() {
                state.exact_piles = true;
            }
            for card in answer {
                if matches!(selector.operation, Operation::ApplySingleTurnSly) {
                    state.card_states.set_transient_sly(card.uid);
                } else if matches!(selector.operation, Operation::ApplyPermanentEthereal) {
                    let mut instance = state.card_states.get(card.uid);
                    instance.set_local_ethereal(true);
                    state.card_states.set(card.uid, instance);
                } else {
                    state.card_states.set_local_retain(card.uid);
                }
            }
            Ok(())
        }
        Operation::Move { destination, top } => {
            for card in answer {
                if state.history.over {
                    break;
                }
                let card = remove_uid(state, selector.pile, card.uid)?;
                let destination =
                    if destination == PileId::Hand && state.piles.get(PileId::Hand).len() >= 10 {
                        PileId::Discard
                    } else {
                        destination
                    };
                state
                    .frames
                    .repair_card_play_after_move(card.uid, selector.pile, destination)
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
                if top {
                    state.piles.get_mut(destination).make_mut().insert(0, card);
                } else {
                    state.piles.get_mut(destination).make_mut().push(card);
                }
                events.push(Event::CardResolved {
                    uid: card.uid,
                    pile: destination,
                });
            }
            Ok(())
        }
        Operation::CloneGenerated { count } => {
            let [frozen] = answer else {
                return if answer.is_empty() {
                    // `_pick_subsets` clamps its lower bound to the live
                    // candidate count. An exact-one selector therefore
                    // completes as a no-op when no Colorless card exists.
                    Ok(())
                } else {
                    Err(EngineRefusal::MalformedArgs("clone selection count"))
                };
            };
            let matches: Vec<_> = state
                .piles
                .get(selector.pile)
                .as_slice()
                .iter()
                .filter(|live| snapshot_matches_live(**live, *frozen))
                .copied()
                .collect();
            let [source] = matches.as_slice() else {
                return Err(EngineRefusal::FrozenCardVanished {
                    uid: frozen.uid,
                    pile: selector.pile,
                });
            };
            // Python `_apply_select_op` (frozen, deleted #2827) delegates
            // `clone_generated` to the shared generated-card transaction
            // without removing its selected physical source. Rehearse locally
            // so a direct helper call also cannot publish history/listener/
            // allocator prefixes.
            //
            // Native Heirloom Hammer (count 1) and Dual Wield (count 1 + upgrade)
            // call `CreateClone` on the selected card once per
            // `AddGeneratedCardToCombat` (Dual Wield `<OnPlay>d__5` RVA
            // `0x39aa54` IL_00e4-IL_0181), so each pass re-reads the live
            // source by UID: a listener run by an earlier Add can never leave
            // a stale payload in a later clone.
            fn clone_passes(
                state: &mut HotState,
                catalog: &Catalog,
                pile: PileId,
                source_uid: u32,
                count: usize,
                events: &mut Vec<Event>,
            ) -> Result<(), EngineRefusal> {
                for _ in 0..count {
                    let source = state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .copied()
                        .find(|live| live.uid == source_uid)
                        .ok_or(EngineRefusal::FrozenCardVanished {
                            uid: source_uid,
                            pile,
                        })?;
                    crate::engine::cards::inject_generated_clones_bottom(
                        state,
                        catalog,
                        source,
                        1,
                        PileId::Hand,
                        events,
                    )?;
                }
                Ok(())
            }
            let mut probe = state.clone();
            clone_passes(
                &mut probe,
                catalog,
                selector.pile,
                source.uid,
                count,
                &mut Vec::new(),
            )?;
            clone_passes(state, catalog, selector.pile, source.uid, count, events)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::engine::play::autoplay_collected_cards;
    use crate::engine::{
        Action, LegalActionBuffer, SelectionAnswer, SelectionRef, apply_action, apply_action_into,
        legal_actions, legal_actions_into,
    };
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_GENETIC_ALGORITHM_STATE, CARD_FLAG_LEGACY,
        CARD_FLAG_RINGING, CardInstanceState, GeneticAlgorithmState, HotMonster, LEGACY_CARD_UID,
        LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, RngStream, RngStreamState,
    };
    use crate::ids::{CardId, MonsterKind, PowerId};
    use crate::powers::SlotWire;

    /// #3125: `recorded_selection_answer`'s matching rule, branch by branch,
    /// over a fixed decode table (index = ordinal).
    #[test]
    fn unique_recorded_answer_keeps_the_one_applying_exact_match() {
        let table: Vec<Result<Option<Vec<u32>>, EngineRefusal>> = vec![
            Ok(Some(vec![5, 6])),
            Ok(Some(vec![6, 5])),
            Ok(None),
            Err(EngineRefusal::MalformedArgs("selection option index")),
            Ok(Some(vec![7])),
            Ok(Some(vec![7])),
            Ok(Some(vec![8])),
            Ok(Some(vec![8])),
        ];
        let resolve = |uids: &[u32], refused: &[u32]| {
            unique_recorded_answer(
                (0..table.len() as u32).map(SelectionAnswer::OptionIndex),
                uids,
                |answer| {
                    let SelectionAnswer::OptionIndex(index) = answer else {
                        unreachable!()
                    };
                    table[index as usize].clone()
                },
                |answer| {
                    let SelectionAnswer::OptionIndex(index) = answer else {
                        unreachable!()
                    };
                    !refused.contains(&index)
                },
            )
        };
        // Order is part of the match.
        assert_eq!(
            resolve(&[6, 5], &[]),
            Ok(Some(SelectionAnswer::OptionIndex(1)))
        );
        // Nonphysical and refused decodes name nothing.
        assert_eq!(resolve(&[9], &[]), Ok(None));
        // A matching answer the engine refuses to apply is not the answer.
        assert_eq!(resolve(&[5, 6], &[0]), Ok(None));
        assert_eq!(
            resolve(&[7], &[4]),
            Ok(Some(SelectionAnswer::OptionIndex(5)))
        );
        // Two applying matches refuse by name rather than pick one.
        assert_eq!(
            resolve(&[8], &[]),
            Err(EngineRefusal::MalformedArgs(
                "recorded selection names more than one answer"
            ))
        );
    }

    fn identity(id: CardId) -> CardIdentity {
        CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        }
    }

    fn card(catalog: &Catalog, id: CardId, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog.atom(&identity(id)).unwrap(),
            flags: 0,
        }
    }

    fn leveled_card(catalog: &Catalog, id: CardId, upgrade: u8, uid: u32) -> HotCard {
        HotCard {
            uid,
            atom: catalog
                .atom(&CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                })
                .unwrap(),
            flags: 0,
        }
    }

    #[test]
    fn hand_trick_temporary_sly_survives_selection_reload_and_survivor_discard() {
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            builder
                .intern_reachable(CardIdentity {
                    id: CardId::HandTrick,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            for id in [CardId::Survivor, CardId::DefendSilent] {
                builder.intern_reachable(identity(id)).unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build().with_action_replay_required();
            let mut state = HotState::at_defaults();
            state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            state.hp = 70;
            state.max_hp = 70;
            state.energy = 3;
            state.next_card_uid = 4;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().extend([
                leveled_card(&catalog, CardId::HandTrick, upgrade, 1),
                card(&catalog, CardId::Survivor, 2),
                card(&catalog, CardId::DefendSilent, 3),
            ]);
            let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            crate::engine::admit(&doc, &state, &catalog).unwrap();
            let pending = apply_action(
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
            assert_eq!(pending.block, 7 + 3 * i32::from(upgrade));
            let doc = HotBoundary::try_to_canonical(&pending, &catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let pending = HotBoundary::from_canonical(&doc, &catalog).unwrap();
            crate::engine::admit(&doc, &pending, &catalog).unwrap();
            let marked = legal_actions(&pending, &catalog)
                .into_iter()
                .map(|action| apply_action(&pending, &catalog, &action).unwrap().state)
                .find(|s| s.card_states.get(3).transient_sly())
                .expect("Defend selection");
            assert!(!marked.card_states.get(3).local_sly);
            assert_eq!(marked.energy, 2);
            assert_eq!(marked.rng, state.rng);
            let doc = HotBoundary::try_to_canonical(&marked, &catalog).unwrap();
            let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let marked = HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
            crate::engine::admit(&doc, &marked, &rebuilt).unwrap();
            assert!(marked.card_states.get(3).transient_sly());
            let after = apply_action(
                &marked,
                &rebuilt,
                &Action::Play {
                    uid: 2,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert!(after.pending.is_none(), "only Defend remains in Hand");
            assert_eq!(after.block, 7 + 3 * i32::from(upgrade) + 8 + 5);
            assert_eq!(after.energy, 1, "the discarded Defend is free");
            assert_eq!(after.history.card_plays_finished_combat, 3);
            assert_eq!(after.history.discarded_cards_this_turn, 1);
            assert!(
                after.card_states.get(3).transient_sly(),
                "playing does not clear single-turn Sly"
            );
            assert_eq!(after.rng, state.rng);
            assert!(
                after
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 3)
            );
        }
    }

    #[test]
    fn hand_trick_nested_survivor_sly_choices_reload_and_finish_once() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::HandTrick, CardId::Survivor, CardId::DefendSilent] {
            builder.intern_reachable(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let mut catalog = builder.build().with_action_replay_required();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        state.energy = 5;
        state.next_card_uid = 7;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        for (uid, id) in [
            (1, CardId::HandTrick),
            (2, CardId::HandTrick),
            (3, CardId::Survivor),
            (4, CardId::Survivor),
            (5, CardId::DefendSilent),
            (6, CardId::DefendSilent),
        ] {
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(&catalog, id, uid));
        }
        for action in [
            Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::Select {
                answer: crate::engine::SelectionAnswer::CardUid(4),
            },
            Action::Play {
                uid: 2,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::Select {
                answer: crate::engine::SelectionAnswer::CardUid(5),
            },
            Action::Play {
                uid: 3,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::Select {
                answer: crate::engine::SelectionAnswer::CardUid(4),
            },
            Action::Select {
                answer: crate::engine::SelectionAnswer::CardUid(5),
            },
        ] {
            let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
            state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
            crate::engine::admit(&doc, &state, &catalog).unwrap();
            let action = if let Action::Select {
                answer: crate::engine::SelectionAnswer::CardUid(uid),
            } = action
            {
                legal_actions(&state, &catalog).into_iter().find(|candidate| {
                    matches!(candidate, Action::Select { answer } if selected_card_uids(&state, &catalog, *answer).unwrap() == Some(vec![uid]))
                }).expect("the requested physical card is a legal choice")
            } else {
                action
            };
            state = apply_action(&state, &catalog, &action)
                .unwrap_or_else(|e| panic!("{action:?}: {e:?}"))
                .state;
        }
        assert!(state.pending.is_none());
        assert!(state.frames.is_empty());
        assert_eq!(state.energy, 2);
        assert_eq!(state.block, 35); // Hand Trick twice, Survivor twice, Defend once.
        assert_eq!(state.history.card_plays_finished_combat, 5);
        assert_eq!(state.history.discarded_cards_this_turn, 2);
        assert!(state.card_states.get(4).transient_sly());
        assert!(state.card_states.get(5).transient_sly());
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|c| c.uid)
                .collect::<Vec<_>>(),
            [6]
        );
        let ended = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(!ended.card_states.get(4).transient_sly());
        assert!(!ended.card_states.get(5).transient_sly());
    }

    #[test]
    fn hand_trick_filter_and_source_are_exact() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::HandTrick,
            CardId::DefendSilent,
            CardId::Reflex,
            CardId::StrikeSilent,
            CardId::Footwork,
        ] {
            builder.intern_reachable(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let owner = catalog
            .spec(catalog.atom(&identity(CardId::HandTrick)).unwrap())
            .unwrap();
        let step = catalog
            .steps(owner)
            .iter()
            .find(|step| step.kind == StepKind::Select)
            .unwrap();
        let args = catalog.args(step.args);
        let selector = selector_from_args(&catalog, owner, args).unwrap();
        let mut state = HotState::at_defaults();
        for (uid, id) in [
            (1, CardId::DefendSilent),
            (2, CardId::DefendSilent),
            (3, CardId::DefendSilent),
            (4, CardId::Reflex),
            (5, CardId::StrikeSilent),
            (6, CardId::Footwork),
        ] {
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(&catalog, id, uid));
        }
        state.card_states.set_local_sly(2);
        state.card_states.set_transient_sly(3);
        assert_eq!(
            candidates(&state, &catalog, selector)
                .unwrap()
                .iter()
                .map(|c| c.uid)
                .collect::<Vec<_>>(),
            [1]
        );
        for (index, value) in [
            (0, CompiledArg::Pile(PileId::Draw)),
            (1, CompiledArg::I(0)),
            (2, CompiledArg::I(2)),
            (3, CompiledArg::Nil),
        ] {
            let mut changed = args.to_vec();
            changed[index] = value;
            assert!(selector_from_args(&catalog, owner, &changed).is_err());
        }
        for id in [CardId::DefendSilent, CardId::Survivor] {
            let mut forged = *owner;
            forged.identity.id = id;
            assert!(selector_from_args(&catalog, &forged, args).is_err());
        }
        let mut forged = *owner;
        forged.identity.upgrade = 1;
        assert!(selector_from_args(&catalog, &forged, args).is_err());
    }

    /// The canonical two-card discard programs — Hidden Daggers L0/L1
    /// (`discard`) and Prepared L1 (`discard_all`) — enumerate every pick
    /// order (#2675, #2524); their one-card siblings are unchanged.
    #[test]
    fn multi_pick_discard_programs_enumerate_pick_orders() {
        for (id, upgrade, expected) in [
            (CardId::HiddenDaggers, 0, 6),
            (CardId::HiddenDaggers, 1, 6),
            (CardId::Prepared, 1, 6),
            (CardId::Prepared, 0, 3),
        ] {
            let owner_identity = crate::catalog::CardIdentity {
                id,
                upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            builder.intern(owner_identity).unwrap();
            builder.intern(identity(CardId::DefendSilent)).unwrap();
            let catalog = builder.build();
            let owner = catalog
                .spec(catalog.atom(&owner_identity).unwrap())
                .unwrap();
            let step = catalog
                .steps(owner)
                .iter()
                .find(|step| step.kind == StepKind::Select)
                .unwrap();
            let selector = selector_from_args(&catalog, owner, catalog.args(step.args)).unwrap();
            assert_eq!(selector.operation, Operation::Discard);
            assert!(selection_order_is_observable(selector.operation));
            let mut state = HotState::at_defaults();
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .extend((1..=3).map(|uid| card(&catalog, CardId::DefendSilent, uid)));
            // Three candidates choose two in both orders (6), or one (3).
            let options = options(&state, &catalog, selector).unwrap();
            assert_eq!(options.len(), expected, "{id:?}+{upgrade}");
            if expected == 6 {
                // Subset-major: each pair is followed by its reversal.
                let uids = options
                    .iter()
                    .map(|answer| answer.iter().map(|card| card.uid).collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                assert_eq!(
                    uids,
                    [[2, 3], [3, 2], [1, 3], [3, 1], [1, 2], [2, 1]].map(Vec::from)
                );
            }
        }
    }

    /// Every multi-pick Discard/Exhaust selector is order-observable,
    /// whatever its owner (#2524): the sink, not the card, carries the order.
    #[test]
    fn generic_discard_and_exhaust_selectors_enumerate_pick_orders() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=3).map(|uid| card(&catalog, CardId::DefendSilent, uid)));
        for operation in [Operation::Discard, Operation::Exhaust] {
            let selector = |min, max| Selector {
                pile: PileId::Hand,
                min,
                max,
                filter: None,
                operation,
            };
            assert!(selection_order_is_observable(operation));
            // 1 empty + 3 singles + 6 ordered pairs + 6 ordered triples.
            assert_eq!(options(&state, &catalog, selector(0, 3)).unwrap().len(), 16);
            assert_eq!(options(&state, &catalog, selector(2, 2)).unwrap().len(), 6);
            // Single-card selects are unchanged: one answer per card.
            assert_eq!(options(&state, &catalog, selector(1, 1)).unwrap().len(), 3);
        }
    }

    #[test]
    fn transient_keyword_payload_order_and_roundtrip_are_exact() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_reachable(identity(CardId::DefendSilent))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let mut cards = Vec::new();
        // [], [Retain], [Retain,Sly], [Sly] is lexicographic tuple order.
        for (index, (retain, sly)) in [(false, false), (true, false), (true, true), (false, true)]
            .into_iter()
            .enumerate()
        {
            let card = card(&catalog, CardId::DefendSilent, index as u32 + 1);
            let mut instance = CardInstanceState {
                transient_retain: retain,
                ..CardInstanceState::default()
            };
            instance.set_transient_sly(sly);
            state.card_states.set(card.uid, instance);
            cards.push(card);
        }
        state.next_card_uid = 5;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend(cards.iter().copied());
        for (i, left) in cards.iter().enumerate() {
            for (j, right) in cards.iter().enumerate() {
                assert_eq!(
                    card_payload_cmp(&state, &catalog, *left, *right).unwrap(),
                    i.cmp(&j)
                );
            }
        }
        let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let restored = HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
        assert_eq!(restored.card_states, state.card_states);
        let snapshot = state.clone();
        state.card_states.cleanup_transient_keywords();
        assert!(
            cards
                .iter()
                .all(|c| state.card_states.get(c.uid).is_vacant())
        );
        assert!(snapshot.card_states.get(4).transient_sly());
    }

    #[test]
    fn physical_selection_order_preserves_duplicate_and_interleaved_star_rows() {
        use LocalCostExpiration::{ThisCombat as C, ThisTurnOrPlayed as T};
        // Written in canonical lexicographic order, including pairs that the
        // old combat-present/temporary-count comparison incorrectly tied.
        let rows: &[&[LocalCostExpiration]] = &[
            &[],
            &[C],
            &[C, C],
            &[C, T],
            &[C, T, C],
            &[C, T, T],
            &[T],
            &[T, C],
            &[T, C, C],
            &[T, C, T],
            &[T, T],
        ];
        let states: Vec<_> = rows
            .iter()
            .map(|rows| {
                let mut state = CardInstanceState::default();
                state.free_star_cost_this_turn_or_played_rows = state
                    .local_cost_modifiers
                    .set_star_cost_expirations(rows)
                    .unwrap();
                state
            })
            .collect();
        for (left_index, left) in states.iter().enumerate() {
            for (right_index, right) in states.iter().enumerate() {
                assert_eq!(
                    physical_state_cmp(left, right),
                    left_index.cmp(&right_index),
                    "Star rows {:?} versus {:?}",
                    rows[left_index],
                    rows[right_index]
                );
            }
        }
    }

    #[test]
    fn regent_hand_transforms_preserve_selected_order_and_cold_pending_state() {
        for (source_id, replacement_id) in [
            (CardId::Begone, CardId::MinionStrike),
            (CardId::Guards, CardId::MinionSacrifice),
        ] {
            for upgrade in 0..=1 {
                let mut builder = CatalogBuilder::new();
                builder
                    .intern_reachable(CardIdentity {
                        id: source_id,
                        upgrade,
                        enchantment: None,
                    })
                    .unwrap();
                for id in [CardId::DefendIronclad, CardId::DefendSilent] {
                    builder.intern_reachable(identity(id)).unwrap();
                }
                builder.intern_monster(MonsterKind::Toadpole).unwrap();
                let catalog = builder.build().with_action_replay_required();
                let mut state = HotState::at_defaults();
                state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
                state.hp = 50;
                state.max_hp = 50;
                state.energy = 3;
                state.next_card_uid = 4;
                let source = leveled_card(&catalog, source_id, upgrade, 1);
                let mut first = card(&catalog, CardId::DefendIronclad, 2);
                first.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                state.card_states.set_local_retain(2);
                let second = card(&catalog, CardId::DefendSilent, 3);
                state
                    .piles
                    .get_mut(PileId::Hand)
                    .make_mut()
                    .extend([source, first, second]);
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 50));
                let entry = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
                crate::engine::admit(&entry, &state, &catalog).unwrap();
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
                let doc = HotBoundary::try_to_canonical(&suspended, &catalog).unwrap();
                let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
                let restored = HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
                crate::engine::admit(&doc, &restored, &rebuilt).unwrap();
                let owner = rebuilt
                    .spec(
                        rebuilt
                            .atom(&CardIdentity {
                                id: source_id,
                                upgrade,
                                enchantment: None,
                            })
                            .unwrap(),
                    )
                    .unwrap();
                let step = rebuilt
                    .steps(owner)
                    .iter()
                    .find(|step| step.kind == StepKind::Select)
                    .unwrap();
                let selector =
                    selector_from_args(&rebuilt, owner, rebuilt.args(step.args)).unwrap();
                let choices = options(&restored, &rebuilt, selector).unwrap();
                assert_eq!(
                    choices.len(),
                    if source_id == CardId::Guards { 5 } else { 2 }
                );
                assert_eq!(legal_actions(&restored, &rebuilt).len(), choices.len());
                for (index, choice) in choices.iter().enumerate() {
                    let action = select_action(index as u32);
                    let after = apply_action(&restored, &rebuilt, &action).unwrap().state;
                    assert!(after.pending.is_none());
                    assert_eq!(after.next_card_uid, 4 + choice.len() as u32);
                    assert_eq!(
                        after.history.owner_generated_cards_combat,
                        choice.len() as i32
                    );
                    assert_eq!(after.rng, state.rng);
                    assert_eq!(
                        after.ps_strikes,
                        if source_id == CardId::Begone {
                            choice.len() as i32
                        } else {
                            0
                        }
                    );
                    let destination = if source_id == CardId::Guards {
                        PileId::Exhaust
                    } else {
                        PileId::Discard
                    };
                    assert_eq!(after.piles.get(destination).as_slice()[0].uid, 1);
                    let hand = after.piles.get(PileId::Hand).as_slice();
                    assert_eq!(hand.len(), 2);
                    for (position, original) in [first, second].iter().enumerate() {
                        if let Some(serial) = choice
                            .iter()
                            .position(|selected| selected.uid == original.uid)
                        {
                            assert_eq!(hand[position].uid, 4 + serial as u32);
                            assert_eq!(
                                rebuilt.spec(hand[position].atom).unwrap().identity,
                                CardIdentity {
                                    id: replacement_id,
                                    upgrade,
                                    enchantment: None
                                }
                            );
                            assert!(after.card_states.get(original.uid).is_vacant());
                            assert!(after.card_states.get(hand[position].uid).is_vacant());
                        } else {
                            assert_eq!(hand[position], *original);
                            assert_eq!(
                                after.card_states.get(original.uid),
                                state.card_states.get(original.uid)
                            );
                        }
                    }
                    if choice.len() == 2 {
                        // First replacement succeeds; second allocation fails.
                        // Public application must publish neither prefix.
                        let mut overflow_entry = state.clone();
                        overflow_entry.next_card_uid = u32::MAX - 1;
                        let overflow = apply_action(
                            &overflow_entry,
                            &catalog,
                            &Action::Play {
                                uid: 1,
                                target: None,
                                selection: SelectionRef::NONE,
                            },
                        )
                        .unwrap()
                        .state;
                        let before = overflow.clone();
                        assert!(matches!(
                            apply_action(&overflow, &catalog, &action),
                            Err(EngineRefusal::CounterOverflow("next_card_uid"))
                        ));
                        assert_eq!(overflow, before);
                    }
                }
            }
        }
    }

    #[test]
    fn charge_transforms_draw_choices_serially_then_upgrades_and_reloads() {
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            let source_id = CardIdentity {
                id: CardId::Charge,
                upgrade,
                enchantment: None,
            };
            builder.intern_reachable(source_id).unwrap();
            let original_ids = [
                CardId::DefendIronclad,
                CardId::DefendSilent,
                CardId::DefendDefect,
            ];
            for id in original_ids {
                builder.intern_reachable(identity(id)).unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build().with_action_replay_required();
            assert!(catalog.atom(&identity(CardId::MinionDiveBomb)).is_some());
            let mut state = HotState::at_defaults();
            state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.next_card_uid = 5;
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(leveled_card(&catalog, CardId::Charge, upgrade, 1));
            for (index, id) in original_ids.into_iter().enumerate() {
                let mut original = card(&catalog, id, index as u32 + 2);
                original.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                state.card_states.set_local_retain(original.uid);
                state.piles.get_mut(PileId::Draw).make_mut().push(original);
            }
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));
            let play = Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            };
            let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            crate::engine::admit(&doc, &state, &catalog).unwrap();
            let suspended = apply_action(&state, &catalog, &play).unwrap().state;
            let doc = HotBoundary::try_to_canonical(&suspended, &catalog).unwrap();
            let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let restored = HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
            crate::engine::admit(&doc, &restored, &rebuilt).unwrap();
            let owner = rebuilt.spec(rebuilt.atom(&source_id).unwrap()).unwrap();
            let args = rebuilt.args(rebuilt.steps(owner)[0].args);
            let selector = selector_from_args(&rebuilt, owner, args).unwrap();
            let choices = options(&restored, &rebuilt, selector).unwrap();
            assert_eq!(choices.len(), 6);
            for (index, choice) in choices.iter().enumerate() {
                let action = select_action(index as u32);
                let after = apply_action(&restored, &rebuilt, &action).unwrap().state;
                for (position, original) in
                    state.piles.get(PileId::Draw).as_slice().iter().enumerate()
                {
                    let result = after.piles.get(PileId::Draw).as_slice()[position];
                    if let Some(serial) = choice.iter().position(|card| card.uid == original.uid) {
                        assert_eq!(result.uid, 5 + serial as u32);
                        assert_eq!(
                            rebuilt.spec(result.atom).unwrap().identity,
                            CardIdentity {
                                id: CardId::MinionDiveBomb,
                                upgrade,
                                enchantment: None,
                            }
                        );
                        assert!(after.card_states.get(result.uid).is_vacant());
                        assert!(after.card_states.get(original.uid).is_vacant());
                    } else {
                        assert_eq!((result.uid, result.flags), (original.uid, original.flags));
                        assert_eq!(
                            rebuilt.spec(result.atom).unwrap().identity,
                            catalog.spec(original.atom).unwrap().identity
                        );
                    }
                }
                assert_eq!(after.next_card_uid, 7);
                assert_eq!(after.history.owner_generated_cards_combat, 2);
                assert_eq!(after.next_generated_hook_uid, 2);
                assert_eq!(after.energy, 2);
                assert_eq!(after.rng, state.rng);
                assert!(after.pending.is_none());
                assert_eq!(after.piles.get(PileId::Discard).as_slice()[0].uid, 1);
            }
            for count in 0..=1 {
                let mut small = state.clone();
                small.piles.get_mut(PileId::Draw).make_mut().truncate(count);
                let after = apply_action(&small, &catalog, &play).unwrap().state;
                assert!(after.pending.is_none());
                assert_eq!(after.history.owner_generated_cards_combat, count as i32);
                assert_eq!(after.next_card_uid, 5 + count as u32);
            }
            let mut overflow = state.clone();
            overflow.next_card_uid = u32::MAX - 1;
            let parked = apply_action(&overflow, &catalog, &play).unwrap().state;
            assert!(matches!(
                apply_action(&parked, &catalog, &select_action(0)),
                Err(EngineRefusal::CounterOverflow("next_card_uid"))
            ));
            for (index, operand) in [
                (0, CompiledArg::Pile(PileId::Hand)),
                (1, CompiledArg::I(1)),
                (2, CompiledArg::I(3)),
                (3, CompiledArg::Filter(FilterMode::Skill)),
            ] {
                let mut forged = args.to_vec();
                forged[index] = operand;
                assert!(selector_from_args(&rebuilt, owner, &forged).is_err());
            }
            let mut forged_owner = *owner;
            forged_owner.identity.id = CardId::Seance;
            assert!(selector_from_args(&rebuilt, &forged_owner, args).is_err());
        }
    }

    #[test]
    fn regent_hand_transform_selector_rejects_forged_owner_and_cardinality() {
        for id in [CardId::Begone, CardId::Guards] {
            let mut builder = CatalogBuilder::new();
            builder.intern_reachable(identity(id)).unwrap();
            let catalog = builder.build();
            let owner = catalog.spec(catalog.atom(&identity(id)).unwrap()).unwrap();
            let step = catalog
                .steps(owner)
                .iter()
                .find(|step| step.kind == StepKind::Select)
                .unwrap();
            let args = catalog.args(step.args);
            assert!(selector_from_args(&catalog, owner, args).is_ok());
            for forged_id in [
                CardId::StrikeIronclad,
                if id == CardId::Begone {
                    CardId::Guards
                } else {
                    CardId::Begone
                },
            ] {
                let mut forged = *owner;
                forged.identity.id = forged_id;
                assert!(selector_from_args(&catalog, &forged, args).is_err());
            }
            let mut forged = *owner;
            forged.identity.upgrade = 1;
            assert!(selector_from_args(&catalog, &forged, args).is_err());
            for (index, value) in [
                (0, CompiledArg::Pile(PileId::Draw)),
                (1, CompiledArg::I(2)),
                (2, CompiledArg::I(2)),
            ] {
                let mut changed = args.to_vec();
                changed[index] = value;
                assert!(selector_from_args(&catalog, owner, &changed).is_err());
            }
        }
    }

    #[test]
    fn seance_transforms_one_draw_card_to_fresh_soul_after_cold_selection_reload() {
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            let seance = CardIdentity {
                id: CardId::Seance,
                upgrade,
                enchantment: None,
            };
            builder.intern_reachable(seance).unwrap();
            builder
                .intern_reachable(CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
            builder
                .intern_reachable(identity(CardId::DefendIronclad))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            assert!(catalog.atom(&identity(CardId::Soul)).is_some());
            let mut state = HotState::at_defaults();
            state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.next_card_uid = 4;
            let source = leveled_card(&catalog, CardId::Seance, upgrade, 1);
            let mut target = leveled_card(&catalog, CardId::StrikeIronclad, 1, 2);
            target.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.card_states.set_local_retain(target.uid);
            let untouched = card(&catalog, CardId::DefendIronclad, 3);
            state.ps_strikes = 1;
            state.piles.get_mut(PileId::Hand).make_mut().push(source);
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend([target, untouched]);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));
            let entry = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            crate::engine::admit(&entry, &state, &catalog).unwrap();
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
            assert_eq!(legal_actions(&suspended, &catalog).len(), 2);
            let doc = HotBoundary::try_to_canonical(&suspended, &catalog).unwrap();
            let rebuilt = HotBoundary::catalog_from_canonical(&doc).unwrap();
            let restored = HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
            crate::engine::admit(&doc, &restored, &rebuilt).unwrap();
            let action = select_action(0);
            let after = apply_action(&restored, &rebuilt, &action).unwrap().state;
            let draw = after.piles.get(PileId::Draw).as_slice();
            assert_eq!(draw.len(), 2);
            assert_eq!(draw[0].uid, 4);
            assert_eq!(
                rebuilt.spec(draw[0].atom).unwrap().identity,
                identity(CardId::Soul)
            );
            assert_eq!(draw[1].uid, untouched.uid);
            assert_eq!(after.ps_strikes, 0);
            assert!(after.card_states.get(2).is_vacant());
            assert!(
                after.card_states.get(4).is_vacant(),
                "new Soul inherits no physical modifiers"
            );
            assert_eq!(after.next_card_uid, 5);
            assert_eq!(after.history.owner_generated_cards_combat, 1);
            assert_eq!(after.next_generated_hook_uid, 1);
            assert_eq!(after.energy, 2 + i16::from(upgrade));
            assert!(after.pending.is_none());
            assert_eq!(after.piles.get(PileId::Discard).as_slice()[0].uid, 1);
            assert_eq!(after.rng, state.rng, "a fixed transform consumes no RNG");

            let mut overflow = restored.clone();
            overflow.next_card_uid = u32::MAX;
            let before = overflow.clone();
            assert!(apply_action(&overflow, &rebuilt, &action).is_err());
            assert_eq!(overflow, before);

            let mut empty = state;
            empty.piles.get_mut(PileId::Draw).make_mut().clear();
            empty.ps_strikes = 0;
            let no_op = apply_action(
                &empty,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert!(no_op.pending.is_none());
            assert_eq!(no_op.next_card_uid, 4);
            assert_eq!(no_op.history.owner_generated_cards_combat, 0);
        }
    }

    #[test]
    fn seance_selector_rejects_forged_owners_and_operands() {
        let mut builder = CatalogBuilder::new();
        builder.intern_reachable(identity(CardId::Seance)).unwrap();
        let catalog = builder.build();
        let owner = catalog
            .spec(catalog.atom(&identity(CardId::Seance)).unwrap())
            .unwrap();
        let args = catalog.args(catalog.steps(owner)[0].args);
        assert!(selector_from_args(&catalog, owner, args).is_ok());
        let mut forged = *owner;
        forged.identity.id = CardId::StrikeIronclad;
        assert!(selector_from_args(&catalog, &forged, args).is_err());
        for (index, value) in [
            (0, CompiledArg::Pile(PileId::Hand)),
            (1, CompiledArg::I(0)),
            (2, CompiledArg::I(2)),
        ] {
            let mut changed = args.to_vec();
            changed[index] = value;
            assert!(selector_from_args(&catalog, owner, &changed).is_err());
        }
    }

    fn armaments_l0_fixture() -> (HotState, Catalog, [HotCard; 3]) {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::Armaments,
            CardId::Apotheosis,
            CardId::StrikeIronclad,
        ] {
            for upgrade in 0..=1 {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade,
                        enchantment: None,
                    })
                    .unwrap();
            }
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let cards = [
            leveled_card(&catalog, CardId::Armaments, 0, 1),
            leveled_card(&catalog, CardId::Apotheosis, 0, 2),
            leveled_card(&catalog, CardId::StrikeIronclad, 0, 3),
        ];
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        (state, catalog, cards)
    }

    fn legacy_card(catalog: &Catalog, id: CardId) -> HotCard {
        HotCard {
            uid: LEGACY_CARD_UID,
            atom: catalog.atom(&identity(id)).unwrap(),
            flags: CARD_FLAG_LEGACY,
        }
    }

    fn assert_buffered_actions_match(state: &HotState, catalog: &Catalog) {
        let expected = legal_actions(state, catalog);
        let mut buffer = LegalActionBuffer::new();
        assert_eq!(legal_actions_into(state, catalog, &mut buffer), expected);
    }

    fn pending_record(state: &HotState) -> crate::hot::CardPlayView<'_> {
        state
            .pending
            .as_deref()
            .and_then(|pending| pending.record(&state.frames))
            .expect("authenticated pending CardPlay")
    }

    fn update_pending_record(
        state: &mut HotState,
        update: impl FnOnce(&mut crate::hot::CardPlayRecord),
    ) {
        let pending = state.pending.as_deref().expect("pending selection");
        let mut record = state
            .frames
            .card_play(pending.frame_record)
            .expect("pending CardPlay record")
            .to_owned();
        update(&mut record);
        state
            .frames
            .replace_top_card_play(&record)
            .expect("valid replacement CardPlay");
    }

    fn survivor_fixture() -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::Survivor, CardId::StrikeSilent, CardId::DefendSilent] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Survivor, 1),
            card(&catalog, CardId::StrikeSilent, 2),
            card(&catalog, CardId::DefendSilent, 3),
        ]);
        (state, catalog)
    }

    #[test]
    fn permanent_retain_operation_uses_the_effective_filter_without_moving() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::SculptingStrike,
            CardId::StrikeSilent,
            CardId::Purity,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        // The fourth effective-Retain source: STEADY's derived keyword must
        // exclude its card from `WithoutEffectiveRetain` exactly as the row
        // flag and the two physical keywords do (Python `card_is_retained`, frozen, deleted #2827, over `card_effective_keywords`).
        let steady_atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeSilent,
                upgrade: 0,
                enchantment: Some(crate::catalog::CardEnchantment {
                    id: crate::ids::EnchantmentId::Steady,
                    amount: 1,
                }),
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let ordinary = card(&catalog, CardId::StrikeSilent, 1);
        let local = card(&catalog, CardId::StrikeSilent, 2);
        let transient = card(&catalog, CardId::StrikeSilent, 3);
        let intrinsic = card(&catalog, CardId::Purity, 4);
        let steady = HotCard {
            uid: 5,
            atom: steady_atom,
            flags: 0,
        };
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([ordinary, local, transient, intrinsic, steady]);
        state.card_states.set_local_retain(local.uid);
        state.card_states.set_transient_retain(transient.uid);
        let args = [
            CompiledArg::Pile(PileId::Hand),
            CompiledArg::I(1),
            CompiledArg::I(1),
            CompiledArg::Filter(FilterMode::WithoutEffectiveRetain),
            CompiledArg::Word(StepWord::ApplyPermanentRetain),
        ];
        let owner = catalog
            .spec(catalog.atom(&identity(CardId::SculptingStrike)).unwrap())
            .unwrap();
        let selector = selector_from_args(&catalog, owner, &args).unwrap();

        assert_eq!(candidates(&state, &catalog, selector).unwrap(), [ordinary]);
        let no_answer = state.clone();
        apply(&mut state, &catalog, selector, &[], &mut Vec::new()).unwrap();
        assert_eq!(state, no_answer, "an empty selection publishes no commit");
        let before_piles = state.piles.clone();
        apply(&mut state, &catalog, selector, &[ordinary], &mut Vec::new()).unwrap();
        assert_eq!(state.piles, before_piles);
        assert!(state.card_states.get(ordinary.uid).local_retain);
        assert!(
            state.exact_piles,
            "the physical keyword write forces exact mode"
        );

        let snapshot = state.clone();
        let vanished = HotCard {
            uid: 99,
            ..ordinary
        };
        assert_eq!(
            apply(&mut state, &catalog, selector, &[vanished], &mut Vec::new()),
            Err(EngineRefusal::FrozenCardVanished {
                uid: 99,
                pile: PileId::Hand,
            })
        );
        assert_eq!(state, snapshot);
    }

    #[test]
    fn sculpting_strike_dispatches_the_permanent_ethereal_writer() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::SculptingStrike, CardId::DefendSilent] {
            builder.intern(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::SculptingStrike, 1),
            card(&catalog, CardId::DefendSilent, 2),
        ]);

        let out = apply_action(
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

        assert_eq!(out.piles.get(PileId::Hand).as_slice()[0].uid, 2);
        // #3022: SculptingStrike OnPlay RVA 0x3b8cec IL_0187 is Ethereal (2).
        assert!(out.card_states.get(2).local_ethereal());
        assert!(!out.card_states.get(2).local_retain);
        assert!(!out.card_states.get(2).transient_retain);
        assert!(
            out.exact_piles,
            "the physical keyword write forces exact mode"
        );
        assert!(crate::engine::cards::call_local_ethereal_provenance_is_exact(&out, &catalog));
    }

    /// `<>c::<OnPlay>b__7_0` RVA 0x3b8cda reads `GetKeywordsWithSources(2)`
    /// = `LocalKeywords`: a canonical or added Ethereal excludes the card, a
    /// Hex combat-time Ethereal (`ModifyKeywordsInCombat`, skipped for flag 2)
    /// does not.
    #[test]
    fn without_ethereal_keyword_reads_local_keywords_not_hex() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::SculptingStrike, CardId::DefendSilent, CardId::Dazed] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let plain = card(&catalog, CardId::DefendSilent, 1);
        let local = card(&catalog, CardId::DefendSilent, 2);
        let canonical = card(&catalog, CardId::Dazed, 3);
        let mut hexed = card(&catalog, CardId::DefendSilent, 4);
        hexed.flags |= CARD_FLAG_HEXED;
        state.powers.set(
            crate::ids::PowerId::HexPower,
            crate::powers::SlotWire::Int,
            2,
        );
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([plain, local, canonical, hexed]);
        let mut instance = state.card_states.get(local.uid);
        instance.set_local_ethereal(true);
        state.card_states.set(local.uid, instance);
        let hex_spec = catalog.spec(hexed.atom).unwrap();
        assert!(
            crate::engine::cards::effective_ethereal(&state, hex_spec, hexed.uid),
            "the Hex card is effectively Ethereal, yet native still offers it"
        );

        let args = [
            CompiledArg::Pile(PileId::Hand),
            CompiledArg::I(1),
            CompiledArg::I(1),
            CompiledArg::Filter(FilterMode::WithoutEtherealKeyword),
            CompiledArg::Word(StepWord::ApplyPermanentEthereal),
        ];
        let owner = catalog
            .spec(catalog.atom(&identity(CardId::SculptingStrike)).unwrap())
            .unwrap();
        let selector = selector_from_args(&catalog, owner, &args).unwrap();
        assert_eq!(selector.operation, Operation::ApplyPermanentEthereal);
        assert_eq!(
            candidates(&state, &catalog, selector).unwrap(),
            [plain, hexed]
        );

        apply(&mut state, &catalog, selector, &[hexed], &mut Vec::new()).unwrap();
        assert!(state.card_states.get(hexed.uid).local_ethereal());
        assert!(!state.card_states.get(plain.uid).local_ethereal());
    }

    /// The Ethereal writer and its filter are bound to the exact Sculpting
    /// Strike row; the same words under another owner are an I5 refusal.
    #[test]
    fn apply_permanent_ethereal_is_bound_to_sculpting_strike() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::SculptingStrike, CardId::Snap] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let spec = |id| catalog.spec(catalog.atom(&identity(id)).unwrap()).unwrap();
        let ethereal = [
            CompiledArg::Pile(PileId::Hand),
            CompiledArg::I(1),
            CompiledArg::I(1),
            CompiledArg::Filter(FilterMode::WithoutEtherealKeyword),
            CompiledArg::Word(StepWord::ApplyPermanentEthereal),
        ];
        assert_eq!(
            selector_from_args(&catalog, spec(CardId::Snap), &ethereal).unwrap_err(),
            EngineRefusal::MalformedArgs("select filter owner")
        );
        let mut wrong_filter = ethereal;
        wrong_filter[3] = CompiledArg::Filter(FilterMode::WithoutEffectiveRetain);
        assert_eq!(
            selector_from_args(&catalog, spec(CardId::SculptingStrike), &wrong_filter).unwrap_err(),
            EngineRefusal::MalformedArgs("select operation")
        );
        let mut two = ethereal;
        two[2] = CompiledArg::I(2);
        assert_eq!(
            selector_from_args(&catalog, spec(CardId::SculptingStrike), &two).unwrap_err(),
            EngineRefusal::MalformedArgs("select operation")
        );
    }

    #[test]
    fn armaments_l0_manual_selection_round_trips_and_upgrades_exactly_one_card() {
        let (mut state, catalog, [source, apotheosis, strike]) = armaments_l0_fixture();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis, strike]);

        let suspended = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Program);
        assert_eq!(
            record.next_step, 2,
            "Block and Select cursor are precommitted"
        );
        assert_eq!(suspended.block, 5);
        assert_eq!(suspended.energy, 2);
        assert_eq!(suspended.piles.get(PileId::Play).as_slice(), &[source]);
        assert_buffered_actions_match(&suspended, &catalog);
        let actions = legal_actions(&suspended, &catalog);
        assert_eq!(actions, [select_action(0), select_action(1)]);

        let canonical = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            canonical.player["pending"],
            serde_json::json!([
                "frame_select",
                source.uid,
                ["select", "hand", 1, 1, "upgradable", "upgrade"]
            ])
        );
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(reloaded, suspended);

        let first = apply_action(&reloaded, &catalog, &actions[0])
            .unwrap()
            .state;
        let second = apply_action(&reloaded, &catalog, &actions[1])
            .unwrap()
            .state;
        for resolved in [&first, &second] {
            assert!(resolved.pending.is_none());
            assert!(resolved.frames.is_empty());
            assert_eq!(resolved.block, 5, "resume does not replay the prefix");
            assert_eq!(resolved.piles.get(PileId::Discard).as_slice(), &[source]);
            let levels = resolved
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| (card.uid, catalog.spec(card.atom).unwrap().identity.upgrade))
                .collect::<Vec<_>>();
            assert_eq!(levels.iter().filter(|(_, level)| *level == 1).count(), 1);
            assert_eq!(
                levels.iter().map(|(uid, _)| *uid).collect::<Vec<_>>(),
                vec![apotheosis.uid, strike.uid],
                "upgrading a selected card preserves its Hand position"
            );
        }
        assert_ne!(
            first.piles, second.piles,
            "the two answer ordinals stay distinct"
        );
    }

    #[test]
    fn armaments_l0_collected_autoplay_refuses_before_prefix_atomically() {
        let (mut state, catalog, [source, apotheosis, strike]) = armaments_l0_fixture();
        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([apotheosis, strike]);
        let mut events = Vec::new();
        let before = state.clone();

        assert_eq!(
            autoplay_collected_cards(&mut state, &catalog, &[source], &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn armaments_l0_empty_and_singleton_domains_auto_complete_without_pending() {
        let (state, catalog, [source, apotheosis, _]) = armaments_l0_fixture();

        let mut empty = state.clone();
        empty.piles.get_mut(PileId::Hand).make_mut().push(source);
        let empty = apply_action(
            &empty,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(empty.pending.is_none());
        assert!(empty.frames.is_empty());
        assert_eq!(empty.block, 5);
        assert_eq!(empty.piles.get(PileId::Discard).as_slice(), &[source]);

        let mut singleton = state;
        singleton
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis]);
        let singleton = apply_action(
            &singleton,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(singleton.pending.is_none());
        assert!(singleton.frames.is_empty());
        assert_eq!(singleton.block, 5);
        assert_eq!(singleton.piles.get(PileId::Discard).as_slice(), &[source]);
        assert_eq!(
            catalog
                .spec(singleton.piles.get(PileId::Hand).as_slice()[0].atom)
                .unwrap()
                .identity,
            CardIdentity {
                id: CardId::Apotheosis,
                upgrade: 1,
                enchantment: None,
            }
        );
    }

    #[test]
    fn armaments_l0_hand_autoplay_refuses_before_prefix_atomically() {
        let (mut state, catalog, [source, apotheosis, strike]) = armaments_l0_fixture();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis, strike]);
        let mut events = Vec::new();
        let before = state.clone();

        assert_eq!(
            autoplay_collected_cards(&mut state, &catalog, &[source], &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn armaments_l0_missing_successor_refuses_before_prefix_and_autoplay_is_atomic() {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::Armaments)).unwrap();
        let target_atom = builder.intern(identity(CardId::Apotheosis)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: source_atom,
            flags: 0,
        };
        let target = HotCard {
            uid: 2,
            atom: target_atom,
            flags: 0,
        };
        let missing = CardIdentity {
            id: CardId::Apotheosis,
            upgrade: 1,
            enchantment: None,
        };
        let mut manual = HotState::at_defaults();
        manual.hp = 50;
        manual.energy = 3;
        manual.next_card_uid = 3;
        manual
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        manual
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, target]);
        let before = manual.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &manual,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                &mut events,
            ),
            Err(EngineRefusal::UnknownMintIdentity(missing))
        );
        assert_eq!(manual, before);
        assert!(
            events.is_empty(),
            "no spend, source move, Block, or event escaped"
        );

        let mut imported_pending_view = HotState::at_defaults();
        imported_pending_view.next_card_uid = 3;
        imported_pending_view
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(target);
        assert_eq!(
            candidates(
                &imported_pending_view,
                &catalog,
                Selector {
                    pile: PileId::Hand,
                    min: 1,
                    max: 1,
                    filter: Some(FilterMode::Upgradable),
                    operation: Operation::Upgrade,
                },
            ),
            Err(EngineRefusal::UnknownMintIdentity(missing)),
            "option enumeration must not advertise a native candidate whose writer cannot resolve"
        );

        let mut autoplay = before;
        autoplay.piles.get_mut(PileId::Hand).make_mut().remove(0);
        autoplay
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let before = autoplay.clone();
        assert_eq!(
            autoplay_collected_cards(&mut autoplay, &catalog, &[source], &mut events),
            Err(EngineRefusal::UnknownMintIdentity(missing))
        );
        assert_eq!(autoplay, before);
        assert!(events.is_empty());
    }

    #[test]
    fn armaments_l0_uses_the_existing_frozen_continuation_layout() {
        assert_eq!(std::mem::size_of::<crate::frame::Frame>(), 8);
        assert_eq!(std::mem::size_of::<crate::hot::Frames>(), 8);
        assert_eq!(std::mem::size_of::<crate::hot::PendingWord>(), 8);
        assert_eq!(std::mem::size_of::<crate::hot::PendingSelection>(), 8);
        assert_eq!(std::mem::size_of::<HotState>(), 224);
    }

    fn select_action(index: u32) -> Action {
        Action::Select {
            answer: SelectionAnswer::OptionIndex(index),
        }
    }

    #[test]
    fn dredge_suspends_round_trips_and_resumes_one_exact_three_card_move() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::Dredge, CardId::StrikeIronclad] {
            builder.intern(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 6;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::Dredge, 1));
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..=5).map(|uid| card(&catalog, CardId::StrikeIronclad, uid)));

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
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::HandCap);
        assert_buffered_actions_match(&suspended, &catalog);
        assert_eq!(record.next_step, 1);
        assert_eq!(record.selection_amount, 3);
        assert!(record.selection_cards().is_empty());
        let actions = crate::engine::legal_actions(&suspended, &catalog);
        // Four-choose-three sub-multisets, each in every one of its 3! pick
        // orders: the plural `CardPileCmd::Add` moves the selected cards to
        // Hand/Bottom in the player's own order, so the order is part of the
        // action (#2521, `selection_order_is_observable`).
        assert_eq!(actions.len(), 24, "four choose three, times 3! orders");

        let canonical = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            canonical.player["pending"],
            serde_json::json!([
                "frame_select",
                pending.frame_uid,
                ["select", "discard", 3, 3, null, ["move", "hand", "bottom"]]
            ])
        );
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(reloaded, suspended);
        assert_eq!(crate::engine::legal_actions(&reloaded, &catalog), actions);

        let mut malformed = canonical.clone();
        malformed
            .player
            .get_mut("pending")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|outer| outer.get_mut(2))
            .and_then(serde_json::Value::as_array_mut)
            .unwrap()[3] = serde_json::json!(2);
        let unchanged = malformed.clone();
        assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());
        assert_eq!(malformed, unchanged, "malformed reload is non-mutating");

        let mut malformed_hot = suspended.clone();
        update_pending_record(&mut malformed_hot, |record| {
            record.source_pile = PileId::Hand
        });
        let unchanged = malformed_hot.clone();
        assert_eq!(
            apply_action(&malformed_hot, &catalog, &actions[0]),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(malformed_hot, unchanged, "malformed action is atomic");

        let resumed = apply_action(&reloaded, &catalog, &actions[0])
            .unwrap()
            .state;
        assert!(resumed.pending.is_none());
        assert!(resumed.frames.is_empty());
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 3);
        assert_eq!(resumed.piles.get(PileId::Discard).len(), 1);
        assert!(
            resumed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );

        // Every one of the 24 answers is a distinct Hand order, and each
        // option ordinal names exactly the order it enumerated.
        let mut hands: Vec<Vec<u32>> = Vec::new();
        for action in &actions {
            let applied = apply_action(&reloaded, &catalog, action).unwrap().state;
            hands.push(
                applied
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect(),
            );
        }
        let mut distinct = hands.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 24, "pick order is observable in the Hand");
        assert_eq!(hands[0], vec![3, 4, 5], "first answer is candidate order");
        assert_eq!(hands[1], vec![3, 5, 4], "index-lexicographic permutations");
    }

    #[test]
    fn neows_fury_attacks_then_offers_empty_through_live_cap() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::NeowsFury, CardId::StrikeIronclad] {
            builder.intern(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::NeowsFury, 1));
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..=3).map(|uid| card(&catalog, CardId::StrikeIronclad, uid)));

        let suspended = apply_action(
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
        assert_eq!(suspended.monsters[0].hp, 30, "attack precedes selection");
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::HandCap);
        assert_buffered_actions_match(&suspended, &catalog);
        assert_eq!(record.selection_amount, 2);
        let actions = crate::engine::legal_actions(&suspended, &catalog);
        assert_eq!(
            actions.len(),
            5,
            "empty, two singles, and the pair in both pick orders"
        );
        // The pair's two orders are two different documents — this is the
        // exact shape of the #2521 replay gap: the recorded human answer
        // `[uid 2, uid 3]` had no name while only the candidate-pile order
        // was enumerated.
        let first = apply_action(&suspended, &catalog, &actions[3])
            .unwrap()
            .state;
        let second = apply_action(&suspended, &catalog, &actions[4])
            .unwrap()
            .state;
        let hand = |state: &HotState| -> Vec<u32> {
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect()
        };
        assert_eq!(hand(&first), vec![2, 3]);
        assert_eq!(hand(&second), vec![3, 2]);

        let resumed = apply_action(&suspended, &catalog, &actions[0])
            .unwrap()
            .state;
        assert!(resumed.pending.is_none());
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 0);
        assert_eq!(resumed.piles.get(PileId::Discard).len(), 2);
        assert!(
            resumed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );
        assert_eq!(resumed.monsters[0].hp, 30, "resume does not replay attack");
    }

    /// #3122 witness: `Abundance/<OnPlay>d__6::MoveNext` (`0x3888ec`
    /// IL_0027-IL_0058) reads the owner's CharacterCardPool and UnlockState,
    /// never an Entropy owner. A Necrobinder root with NO `entropy_card_pool`
    /// (the Rust opening writes none for Necrobinder) plays Abundance through
    /// the body gate, suspends on the native three-card shuffle prefix of the
    /// owner's Power pool, round-trips through the pending decoder, and
    /// resolves through `abundance_candidates`. A CONTRADICTORY Entropy owner
    /// still refuses at the body and at the decoder.
    #[test]
    fn abundance_admits_an_absent_entropy_owner_and_refuses_a_contradictory_one() {
        let pool = crate::content_tables::ABUNDANCE_POWER_POOLS_V1101
            .iter()
            .find_map(|(name, pool)| (*name == "Necrobinder").then_some(*pool))
            .unwrap();
        let mut builder = CatalogBuilder::new();
        let abundance = CardIdentity {
            id: CardId::Abundance,
            upgrade: 0,
            enchantment: None,
        };
        builder.intern(abundance).unwrap();
        for id in pool {
            builder
                .intern(CardIdentity {
                    id: *id,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(1);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.reward_card_pool = Some(RewardPool::Necrobinder);
        state.entropy_card_pool = None;
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&abundance).unwrap(),
            flags: 0,
        });
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };

        let suspended = apply_action(&state, &catalog, &play).unwrap().state;
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Abundance);
        let mut expected = pool.to_vec();
        let mut preview = seeded;
        preview.shuffle(&mut expected).unwrap();
        let offered: Vec<_> = record
            .selection_cards()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity.id)
            .collect();
        assert_eq!(offered, expected[..3]);
        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert!(
            !wire.player.contains_key("entropy_card_pool")
                || wire.player["entropy_card_pool"].is_null()
        );
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );
        let resolved = apply_action(&suspended, &catalog, &select_action(0))
            .unwrap()
            .state;
        let generated = *resolved.piles.get(PileId::Hand).as_slice().last().unwrap();
        assert_eq!(
            catalog.spec(generated.atom).unwrap().identity.id,
            offered[0]
        );

        let mut contradictory = state.clone();
        contradictory.entropy_card_pool = Some(RewardPool::Ironclad);
        assert!(apply_action(&contradictory, &catalog, &play).is_err());
        let mut contradictory_wire = wire;
        contradictory_wire.player.insert(
            "entropy_card_pool".to_owned(),
            serde_json::json!("ironclad"),
        );
        assert!(HotBoundary::from_canonical(&contradictory_wire, &catalog).is_err());
    }

    #[test]
    fn abundance_freezes_three_upgraded_power_choices_and_mints_only_the_answer_free() {
        let pool = crate::content_tables::ABUNDANCE_POWER_POOLS_V1101
            .iter()
            .find_map(|(name, pool)| (*name == "Ironclad").then_some(*pool))
            .unwrap();
        let mut builder = CatalogBuilder::new();
        let abundance = CardIdentity {
            id: CardId::Abundance,
            upgrade: 0,
            enchantment: None,
        };
        builder.intern(abundance).unwrap();
        for id in pool {
            builder
                .intern(CardIdentity {
                    id: *id,
                    upgrade: 1,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        // With Ancient powers excluded, seed one's Ironclad preview is
        // Vicious+, Juggling+, Rupture+. Execute the Vicious leaf below.
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(1);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&abundance).unwrap(),
            flags: 0,
        });

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
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Abundance);
        assert_buffered_actions_match(&suspended, &catalog);
        let mut expected = pool.to_vec();
        let mut preview = seeded;
        preview.shuffle(&mut expected).unwrap();
        let offered: Vec<_> = record
            .selection_cards()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity)
            .collect();
        assert_eq!(
            offered,
            expected[..3]
                .iter()
                .map(|id| CardIdentity {
                    id: *id,
                    upgrade: 1,
                    enchantment: None,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(crate::engine::legal_actions(&suspended, &catalog).len(), 3);
        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(wire.player["pending"][0], "abundance_select");
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        assert_eq!(HotBoundary::to_canonical(&rebuilt, &rebuilt_catalog), wire);

        let vicious_option = offered
            .iter()
            .position(|identity| identity.id == CardId::Vicious)
            .expect("seed-one Abundance preview carries Vicious");
        let selected = offered[vicious_option];
        let resolved = apply_action(
            &suspended,
            &catalog,
            &select_action(u32::try_from(vicious_option).unwrap()),
        )
        .unwrap()
        .state;
        assert!(resolved.pending.is_none());
        assert_eq!(resolved.history.owner_generated_cards_combat, 1);
        assert!(resolved.exact_piles);
        assert_eq!(resolved.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        let generated = *resolved.piles.get(PileId::Hand).as_slice().last().unwrap();
        assert_eq!(catalog.spec(generated.atom).unwrap().identity, selected);
        assert_eq!(
            resolved
                .card_states
                .get(generated.uid)
                .local_cost_modifiers
                .resolve(9),
            0
        );
        let played = apply_action(
            &resolved,
            &catalog,
            &Action::Play {
                uid: generated.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(played.powers.value(PowerId::Vicious), 2);
        assert_eq!(
            played.fanouts.after_power_amount_changed_order(),
            &[PowerId::Vicious]
        );

        let mut full_hand = suspended.clone();
        let filler_atom = catalog.atom(&offered[0]).unwrap();
        full_hand
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((10..20).map(|uid| HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            }));
        full_hand.next_card_uid = 20;
        let overflowed = apply_action(&full_hand, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(overflowed.piles.get(PileId::Hand).len(), 10);
        assert_eq!(overflowed.piles.get(PileId::Discard).as_slice()[0].uid, 20);

        let mut uid_overflow = suspended.clone();
        uid_overflow.next_card_uid = u32::MAX;
        let before = uid_overflow.clone();
        assert_eq!(
            apply_action(&uid_overflow, &catalog, &select_action(0)).unwrap_err(),
            EngineRefusal::CounterOverflow("next_card_uid")
        );
        assert_eq!(uid_overflow, before);

        let mut terminal = suspended.clone();
        terminal.history.over = true;
        terminal.next_card_uid = u32::MAX;
        crate::engine::play::resume_selection(
            &mut terminal,
            &catalog,
            SelectionAnswer::OptionIndex(0),
            &mut Vec::new(),
        )
        .expect("an ended in-flight choice validates but mints nothing");
        assert!(terminal.pending.is_none());
        assert_eq!(terminal.next_card_uid, u32::MAX);
        assert_eq!(terminal.history.owner_generated_cards_combat, 0);
        assert!(terminal.piles.get(PileId::Hand).is_empty());

        let mut replayed = state.clone();
        replayed.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        replayed.player_side_active = true;
        replayed.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut replayed);
        let replay_predecessor = HotBoundary::to_canonical(&replayed, &catalog);
        let replay_catalog = HotBoundary::catalog_from_canonical(&replay_predecessor).unwrap();
        let replayed = HotBoundary::from_canonical(&replay_predecessor, &replay_catalog).unwrap();
        let first = apply_action(
            &replayed,
            &replay_catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(pending_record(&first).plays, 2);
        let replay_wire = HotBoundary::to_canonical(&first, &replay_catalog);
        let replay_catalog = HotBoundary::catalog_from_canonical(&replay_wire).unwrap();
        let first = HotBoundary::from_canonical(&replay_wire, &replay_catalog).unwrap();
        assert!(crate::boundary::action_replay_is_authenticated(
            &replay_wire,
            &first,
            &replay_catalog
        ));
        assert_eq!(
            crate::engine::play::persisted_card_play_stack_is_exact(&first, &replay_catalog),
            Ok(())
        );
        assert_eq!(
            special_option_count(&first, &replay_catalog, first.pending.as_deref().unwrap()),
            Ok(3)
        );
        assert!(!crate::engine::legal_actions(&first, &replay_catalog).is_empty());
        let second = apply_action(&first, &replay_catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(pending_record(&second).play_index, 1);
        let resolved = apply_action(&second, &replay_catalog, &select_action(0))
            .unwrap()
            .state;
        assert!(resolved.pending.is_none());
        assert_eq!(resolved.history.owner_generated_cards_combat, 2);
        assert_eq!(resolved.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        assert_eq!(resolved.piles.get(PileId::Hand).len(), 2);
        assert_eq!(
            resolved.rng.get(RngStream::Generation).counter,
            seeded.counter + u64::try_from(2 * (pool.len() - 1)).unwrap()
        );

        let mut malformed = wire;
        malformed.player.get_mut("pending").unwrap()[3][0][1] = serde_json::json!(0);
        assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());
    }

    #[test]
    fn discovery_freezes_owner_shuffle_and_mints_only_the_chosen_physical_card_free() {
        let pool = &crate::content_tables::STOKE_CARD_POOL_V109;
        let discovery = CardIdentity {
            id: CardId::Discovery,
            upgrade: 0,
            enchantment: None,
        };
        let discovery_upgraded = CardIdentity {
            upgrade: 1,
            ..discovery
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(discovery).unwrap();
        builder.intern(discovery_upgraded).unwrap();
        for id in pool {
            builder
                .intern(CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(17);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&discovery).unwrap(),
            flags: 0,
        });

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
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Discovery);
        assert_buffered_actions_match(&suspended, &catalog);
        let mut expected = pool.to_vec();
        let mut preview = seeded;
        preview.shuffle(&mut expected).unwrap();
        let offered: Vec<_> = record
            .selection_cards()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity)
            .collect();
        assert_eq!(
            offered,
            expected[..3]
                .iter()
                .map(|id| CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .collect::<Vec<_>>()
        );
        assert!(record.selection_cards().iter().all(|card| card.uid == 0));
        assert_eq!(crate::engine::legal_actions(&suspended, &catalog).len(), 4);

        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(wire.player["pending"][0], "discovery_select");
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );

        let skipped = apply_action(&suspended, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert!(skipped.pending.is_none());
        assert!(skipped.frames.is_empty());
        assert_eq!(skipped.history.owner_generated_cards_combat, 0);
        assert_eq!(skipped.next_card_uid, 2);
        assert_eq!(skipped.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        assert!(skipped.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            skipped.rng.get(RngStream::Generation),
            suspended.rng.get(RngStream::Generation),
            "skip happens after the full owner-pool shuffle"
        );

        let mut upgraded = state.clone();
        upgraded.piles.get_mut(PileId::Hand).make_mut()[0].atom =
            catalog.atom(&discovery_upgraded).unwrap();
        let upgraded_pending = apply_action(
            &upgraded,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let upgraded_skip = apply_action(&upgraded_pending, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert_eq!(
            upgraded_skip.piles.get(PileId::Discard).as_slice()[0].uid,
            1
        );
        assert_eq!(upgraded_skip.history.owner_generated_cards_combat, 0);
        assert_eq!(upgraded_skip.next_card_uid, 2);

        let mut replayed = state.clone();
        replayed.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut replayed);
        let first = apply_action(
            &replayed,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(pending_record(&first).plays, 2);
        let second = apply_action(&first, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert_eq!(pending_record(&second).play_index, 1);
        let replay_skipped = apply_action(&second, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert!(replay_skipped.pending.is_none());
        assert_eq!(replay_skipped.history.owner_generated_cards_combat, 0);
        assert_eq!(replay_skipped.next_card_uid, 2);
        assert!(replay_skipped.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            replay_skipped.piles.get(PileId::Exhaust).as_slice()[0].uid,
            1
        );
        assert_eq!(
            replay_skipped.rng.get(RngStream::Generation).counter,
            seeded.counter + u64::try_from(2 * (pool.len() - 1)).unwrap()
        );

        assert_eq!(
            apply_action(&suspended, &catalog, &select_action(4)).unwrap_err(),
            EngineRefusal::MalformedArgs("selection option index")
        );

        let selected = offered[1];
        let resolved = apply_action(&suspended, &catalog, &select_action(1))
            .unwrap()
            .state;
        assert!(resolved.pending.is_none());
        assert!(resolved.frames.is_empty());
        assert_eq!(resolved.history.owner_generated_cards_combat, 1);
        assert_eq!(resolved.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        let generated = *resolved.piles.get(PileId::Hand).as_slice().last().unwrap();
        assert_eq!(
            generated.uid, 2,
            "the uidless option carrier is not inserted"
        );
        assert_eq!(catalog.spec(generated.atom).unwrap().identity, selected);
        assert_eq!(
            resolved
                .card_states
                .get(generated.uid)
                .local_cost_modifiers
                .resolve(9),
            0
        );
        assert_eq!(resolved.piles.get(PileId::Hand).len(), 1);

        let mut full_hand = suspended.clone();
        let filler_atom = catalog.atom(&offered[0]).unwrap();
        full_hand
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((10..20).map(|uid| HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            }));
        full_hand.next_card_uid = 20;
        let overflowed = apply_action(&full_hand, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(overflowed.piles.get(PileId::Hand).len(), 10);
        assert_eq!(overflowed.piles.get(PileId::Discard).as_slice()[0].uid, 20);

        let mut uid_overflow = suspended.clone();
        uid_overflow.next_card_uid = u32::MAX;
        let before = uid_overflow.clone();
        assert_eq!(
            apply_action(&uid_overflow, &catalog, &select_action(0)).unwrap_err(),
            EngineRefusal::CounterOverflow("next_card_uid")
        );
        assert_eq!(uid_overflow, before);

        let mut terminal = suspended.clone();
        terminal.history.over = true;
        terminal.next_card_uid = u32::MAX;
        crate::engine::play::resume_selection(
            &mut terminal,
            &catalog,
            SelectionAnswer::OptionIndex(0),
            &mut Vec::new(),
        )
        .expect("an ended Discovery screen validates but mints nothing");
        assert!(terminal.pending.is_none());
        assert_eq!(terminal.next_card_uid, u32::MAX);
        assert_eq!(terminal.history.owner_generated_cards_combat, 0);
        assert!(terminal.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn splash_freezes_owner_excluded_shuffle_and_resumes_skip_choice_and_burst_exactly() {
        let pool = crate::steps::neutral::splash_pool(RewardPool::Ironclad);
        let splash = CardIdentity {
            id: CardId::Splash,
            upgrade: 0,
            enchantment: None,
        };
        let splash_upgraded = CardIdentity {
            upgrade: 1,
            ..splash
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(splash).unwrap();
        builder.intern(splash_upgraded).unwrap();
        for id in pool.iter().copied() {
            for upgrade in 0..=1 {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade,
                        enchantment: None,
                    })
                    .unwrap();
            }
        }
        let catalog = builder.build();
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(17);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.reward_card_pool = Some(RewardPool::Ironclad);
        // Splash reads the physical owner's unlocked card pools, not Entropy.
        state.entropy_card_pool = Some(RewardPool::Silent);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: catalog.atom(&splash).unwrap(),
            flags: 0,
        });

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
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Splash);
        assert_buffered_actions_match(&suspended, &catalog);
        let mut expected = pool.clone();
        let mut preview = seeded;
        preview.shuffle(&mut expected).unwrap();
        let offered: Vec<_> = record
            .selection_cards()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity)
            .collect();
        assert_eq!(
            offered,
            expected[..3]
                .iter()
                .map(|id| CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .collect::<Vec<_>>()
        );
        assert!(record.selection_cards().iter().all(|card| card.uid == 0));
        assert_eq!(crate::engine::legal_actions(&suspended, &catalog).len(), 4);
        assert_eq!(
            suspended.rng.get(RngStream::Generation).counter,
            seeded.counter + u64::try_from(pool.len() - 1).unwrap()
        );

        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(wire.player["pending"][0], "splash_select");
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        assert_eq!(HotBoundary::to_canonical(&rebuilt, &rebuilt_catalog), wire);

        let skipped = apply_action(&suspended, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert!(skipped.pending.is_none());
        assert_eq!(skipped.history.owner_generated_cards_combat, 0);
        assert_eq!(skipped.next_card_uid, 2);
        assert_eq!(skipped.piles.get(PileId::Discard).as_slice()[0].uid, 1);
        assert!(skipped.piles.get(PileId::Hand).is_empty());

        let selected = offered[1];
        let resolved = apply_action(&suspended, &catalog, &select_action(1))
            .unwrap()
            .state;
        assert!(resolved.pending.is_none());
        assert_eq!(resolved.history.owner_generated_cards_combat, 1);
        assert_eq!(resolved.piles.get(PileId::Discard).as_slice()[0].uid, 1);
        let generated = *resolved.piles.get(PileId::Hand).as_slice().last().unwrap();
        assert_eq!(
            generated.uid, 2,
            "the uidless option carrier is not inserted"
        );
        assert_eq!(catalog.spec(generated.atom).unwrap().identity, selected);
        assert_eq!(
            resolved
                .card_states
                .get(generated.uid)
                .local_cost_modifiers
                .resolve(9),
            0
        );

        let mut upgraded = state.clone();
        upgraded.piles.get_mut(PileId::Hand).make_mut()[0].atom =
            catalog.atom(&splash_upgraded).unwrap();
        let upgraded_pending = apply_action(
            &upgraded,
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
            pending_record(&upgraded_pending)
                .selection_cards()
                .iter()
                .all(|card| catalog.spec(card.atom).unwrap().identity.upgrade == 1)
        );

        let mut replayed = state.clone();
        replayed.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut replayed);
        let first = apply_action(
            &replayed,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(pending_record(&first).plays, 2);
        let second = apply_action(&first, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert_eq!(pending_record(&second).play_index, 1);
        let replay_skipped = apply_action(&second, &catalog, &select_action(3))
            .unwrap()
            .state;
        assert!(replay_skipped.pending.is_none());
        assert_eq!(replay_skipped.history.owner_generated_cards_combat, 0);
        assert_eq!(
            replay_skipped.piles.get(PileId::Discard).as_slice()[0].uid,
            1
        );
        assert_eq!(
            replay_skipped.rng.get(RngStream::Generation).counter,
            seeded.counter + u64::try_from(2 * (pool.len() - 1)).unwrap()
        );

        let mut full_hand = suspended.clone();
        let filler_atom = catalog.atom(&offered[0]).unwrap();
        full_hand
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((10..20).map(|uid| HotCard {
                uid,
                atom: filler_atom,
                flags: 0,
            }));
        full_hand.next_card_uid = 20;
        let overflowed = apply_action(&full_hand, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(overflowed.piles.get(PileId::Hand).len(), 10);
        assert_eq!(overflowed.piles.get(PileId::Discard).as_slice()[0].uid, 20);

        let mut uid_overflow = suspended.clone();
        uid_overflow.next_card_uid = u32::MAX;
        let before = uid_overflow.clone();
        assert_eq!(
            apply_action(&uid_overflow, &catalog, &select_action(0)).unwrap_err(),
            EngineRefusal::CounterOverflow("next_card_uid")
        );
        assert_eq!(uid_overflow, before);

        let mut terminal = suspended.clone();
        terminal.history.over = true;
        terminal.next_card_uid = u32::MAX;
        crate::engine::play::resume_selection(
            &mut terminal,
            &catalog,
            SelectionAnswer::OptionIndex(0),
            &mut Vec::new(),
        )
        .expect("an ended Splash screen validates but mints nothing");
        assert!(terminal.pending.is_none());
        assert_eq!(terminal.next_card_uid, u32::MAX);
        assert_eq!(terminal.history.owner_generated_cards_combat, 0);
        assert!(terminal.piles.get(PileId::Hand).is_empty());

        assert_eq!(
            apply_action(&suspended, &catalog, &select_action(4)).unwrap_err(),
            EngineRefusal::MalformedArgs("selection option index")
        );

        let mut wrong_level = wire.clone();
        wrong_level.player.get_mut("pending").unwrap()[3][0][1] = serde_json::json!(1);
        assert!(HotBoundary::from_canonical(&wrong_level, &catalog).is_err());
        let mut duplicate = wire.clone();
        let first_option = duplicate.player["pending"][3][0].clone();
        duplicate.player.get_mut("pending").unwrap()[3][1] = first_option;
        assert!(HotBoundary::from_canonical(&duplicate, &catalog).is_err());
        let mut partial_unlocks = wire;
        partial_unlocks.player.remove("splash_unlock_epochs");
        assert!(HotBoundary::from_canonical(&partial_unlocks, &catalog).is_err());
    }

    #[test]
    fn purity_freezes_hand_enumerates_ordered_prefixes_and_resumes_serially() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::Purity,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Purity, 1),
            legacy_card(&catalog, CardId::StrikeIronclad),
            legacy_card(&catalog, CardId::DefendIronclad),
        ]);

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
        let record = pending_record(&suspended);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Purity);
        assert_buffered_actions_match(&suspended, &catalog);
        assert_eq!(
            record
                .selection_cards()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(
            record
                .selection_cards()
                .iter()
                .all(|card| card.flags & CARD_FLAG_PICK != 0)
        );
        assert!(suspended.exact_piles);
        assert_eq!(suspended.next_card_uid, 4);
        assert!(
            record
                .selection_cards()
                .iter()
                .all(|card| card.flags & CARD_FLAG_LEGACY == 0),
            "native exact-pile promotion assigns identities before freezing"
        );
        let actions = crate::engine::legal_actions(&suspended, &catalog);
        assert_eq!(actions.len(), 5, "empty, two singletons, two ordered pairs");
        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );

        let resumed = apply_action(&suspended, &catalog, &actions[4])
            .unwrap()
            .state;
        assert!(resumed.pending.is_none());
        assert!(resumed.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            resumed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [3, 2, 1],
            "chosen cards exhaust in pick order before Purity routes itself"
        );
    }

    #[test]
    fn seeker_freezes_shuffle_membership_but_offers_live_draw_order() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::SeekerStrike,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
            CardId::Bash,
            CardId::Anger,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 2;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.monsters_mut()[0].loop_pos = 2;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::SeekerStrike, 1));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            legacy_card(&catalog, CardId::StrikeIronclad),
            legacy_card(&catalog, CardId::DefendIronclad),
            legacy_card(&catalog, CardId::Bash),
            legacy_card(&catalog, CardId::Anger),
        ]);
        let counter_before = state.rng.get(crate::hot::RngStream::Sel).counter;

        let suspended = apply_action(
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
        let record = pending_record(&suspended);
        assert_eq!(
            record.route().unwrap().1,
            PendingSelectionKind::SeekerStrike
        );
        assert_buffered_actions_match(&suspended, &catalog);
        assert_eq!(record.selection_cards().len(), 3);
        assert_eq!(suspended.next_card_uid, 6);
        assert!(
            suspended
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .all(|card| card.flags & CARD_FLAG_LEGACY == 0),
            "Seeker normalizes the complete Draw pile before StableShuffle"
        );
        assert_eq!(
            suspended.rng.get(crate::hot::RngStream::Sel).counter,
            counter_before + 3,
            "StableShuffle consumes one draw per non-final card"
        );
        let live_options: Vec<_> = suspended
            .piles
            .get(PileId::Draw)
            .as_slice()
            .iter()
            .filter(|card| {
                record
                    .selection_cards()
                    .iter()
                    .any(|pick| pick.uid == card.uid)
            })
            .map(|card| card.uid)
            .collect();
        assert_eq!(crate::engine::legal_actions(&suspended, &catalog).len(), 3);
        let wire = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            HotBoundary::from_canonical(&wire, &catalog).unwrap(),
            suspended
        );
        assert_eq!(
            crate::engine::admission::admit(&wire, &suspended, &catalog),
            Ok(()),
            "the complete suspended Seeker state is admission-valid"
        );
        let mut auto = wire.clone();
        auto.continuations[0]
            .fields
            .insert("source".to_owned(), serde_json::json!("auto"));
        assert!(
            HotBoundary::from_canonical(&auto, &catalog).is_err(),
            "auto special selections need the larger #1542 continuation stack"
        );

        let mut malformed = wire;
        let pending = malformed
            .player
            .get_mut("pending")
            .unwrap()
            .as_array_mut()
            .unwrap();
        let operation = pending[2].as_array_mut().unwrap();
        let filter = operation[4].as_array_mut().unwrap();
        let uids = filter[1].as_array_mut().unwrap();
        uids[1] = uids[0].clone();
        assert!(HotBoundary::from_canonical(&malformed, &catalog).is_err());

        let resumed = apply_action(&suspended, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(
            resumed.piles.get(PileId::Hand).as_slice()[0].uid,
            live_options[0]
        );
        assert!(resumed.pending.is_none());
        assert_eq!(resumed.piles.get(PileId::Discard).as_slice()[0].uid, 1);
    }

    #[test]
    fn seeker_uses_dotnet_introsort_identity_ties_before_shuffling() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((0..20).map(|uid| {
                let mut tied = card(&catalog, CardId::StrikeIronclad, uid);
                // CompareTo ignores physical payload. Alternate payload
                // shapes so this also rejects a .NET sort with the old,
                // over-specific `card_payload_cmp` key.
                if uid % 3 == 0 {
                    tied.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
                }
                tied
            }));

        let mut sorted = seeker_sorted_draw(&state, &catalog).unwrap();
        assert_eq!(
            sorted.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [
                0, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 18, 19,
            ],
            "n > 16 tied CardModel keys preserve .NET introsort's physical order"
        );

        let live = state.rng.get(crate::hot::RngStream::Sel);
        let mut rng = crate::rng::Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        rng.shuffle(&mut sorted).unwrap();
        assert_eq!(
            sorted
                .iter()
                .take(3)
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [17, 16, 15],
            "the exact tied-card order determines Seeker's physical shortlist"
        );
    }

    #[test]
    fn purity_replay_freezes_each_live_hand_and_bad_answers_are_atomic() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::Purity,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 4;
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Purity, 1),
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
        ]);
        let first = apply_action(
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
        let before_bad = first.clone();
        assert!(apply_action(&first, &catalog, &select_action(99)).is_err());
        assert_eq!(first, before_bad);

        let second = apply_action(&first, &catalog, &select_action(1))
            .unwrap()
            .state;
        let record = pending_record(&second);
        assert_eq!(record.route().unwrap().1, PendingSelectionKind::Purity);
        assert_eq!(record.play_index, 1);
        assert_eq!(
            record
                .selection_cards()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(second.powers.value(PowerId::Burst), 0);
    }

    #[test]
    fn lethal_seeker_still_consumes_the_complete_shuffle_without_selecting() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::SeekerStrike,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
            CardId::Bash,
            CardId::Anger,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 5;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::SeekerStrike, 1));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
            card(&catalog, CardId::Bash, 4),
            card(&catalog, CardId::Anger, 5),
        ]);
        let counter = state.rng.get(crate::hot::RngStream::Sel).counter;

        let ended = apply_action(
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

        assert!(ended.history.over);
        assert!(ended.pending.is_none());
        assert!(ended.exact_piles);
        assert_eq!(
            ended.rng.get(crate::hot::RngStream::Sel).counter,
            counter + 3
        );
        assert_eq!(ended.piles.get(PileId::Draw).len(), 4);
    }

    #[test]
    fn lethal_seeker_can_resume_against_its_unchanged_dead_target() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::SeekerStrike,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
            CardId::Bash,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 6;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::SeekerStrike, 1));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
            card(&catalog, CardId::Bash, 4),
        ]);

        let suspended = apply_action(
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
        assert!(!suspended.history.over);
        assert!(suspended.monsters[0].hp <= 0);
        let dead_target_uid = suspended.monsters[0].uid;
        let record = pending_record(&suspended);
        assert_eq!(record.target.map(|target| target.0), Some(0));
        assert_eq!(record.target.map(|target| target.1), Some(dead_target_uid));
        assert_eq!(
            record.route().unwrap().1,
            PendingSelectionKind::SeekerStrike
        );

        let resumed = apply_action(&suspended, &catalog, &select_action(0))
            .expect("the unchanged dead CardPlay target remains authentic")
            .state;
        assert!(resumed.pending.is_none());
        assert_eq!(resumed.monsters[0].uid, dead_target_uid);
        assert!(resumed.monsters[0].hp <= 0);
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 1);
        assert_eq!(resumed.piles.get(PileId::Discard).as_slice()[0].uid, 1);
    }

    #[test]
    fn seeker_zero_one_and_two_draw_boundaries_match_native_auto_take() {
        for count in 0..=2 {
            let mut builder = CatalogBuilder::new();
            for id in [
                CardId::SeekerStrike,
                CardId::StrikeIronclad,
                CardId::DefendIronclad,
            ] {
                builder.intern(identity(id)).unwrap();
            }
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.energy = 3;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Hand).make_mut().push(card(
                &catalog,
                CardId::SeekerStrike,
                1,
            ));
            for (offset, id) in [CardId::StrikeIronclad, CardId::DefendIronclad]
                .into_iter()
                .take(count)
                .enumerate()
            {
                state.piles.get_mut(PileId::Draw).make_mut().push(card(
                    &catalog,
                    id,
                    offset as u32 + 2,
                ));
            }
            state.next_card_uid = count as u32 + 2;

            let out = apply_action(
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
            assert_eq!(
                out.rng.get(crate::hot::RngStream::Sel).counter,
                count.saturating_sub(1) as u64
            );
            if count < 2 {
                assert!(out.pending.is_none());
                assert_eq!(out.piles.get(PileId::Hand).len(), count);
                assert!(out.piles.get(PileId::Draw).is_empty());
                assert_eq!(out.piles.get(PileId::Discard).as_slice()[0].uid, 1);
            } else {
                assert_eq!(
                    pending_record(&out).route().unwrap().1,
                    PendingSelectionKind::SeekerStrike
                );
                assert_eq!(crate::engine::legal_actions(&out, &catalog).len(), 2);
            }
        }
    }

    #[test]
    fn seeker_replay_reshuffles_the_fresh_draw_after_the_first_pick() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::SeekerStrike,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
            CardId::Bash,
            CardId::Anger,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 6;
        state.powers.set(PowerId::OneTwoPunch, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::SeekerStrike, 1));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
            card(&catalog, CardId::Bash, 4),
            card(&catalog, CardId::Anger, 5),
        ]);
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
        let first_shortlist = pending_record(&first).selection_cards().to_vec();

        let second = apply_action(&first, &catalog, &select_action(0))
            .unwrap()
            .state;
        let record = pending_record(&second);
        assert_eq!(
            record.route().unwrap().1,
            PendingSelectionKind::SeekerStrike
        );
        assert_eq!(record.play_index, 1);
        assert_eq!(second.rng.get(crate::hot::RngStream::Sel).counter, 5);
        assert_eq!(second.piles.get(PileId::Draw).len(), 3);
        assert_ne!(record.selection_cards(), first_shortlist);
        assert_eq!(second.powers.value(PowerId::OneTwoPunch), 0);
    }

    #[test]
    fn survivor_parks_exact_remainder_and_resumes_one_discard() {
        let (state, catalog) = survivor_fixture();
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
        let record = pending_record(&suspended);
        assert_eq!(
            record.route().unwrap().1,
            crate::hot::PendingSelectionKind::Program
        );
        assert_buffered_actions_match(&suspended, &catalog);
        assert_eq!(record.next_step, 2);
        assert!(
            !suspended.exact_piles,
            "opening a frame selection does not promote the source state"
        );
        assert_eq!(suspended.block, 8, "prefix ran exactly once");
        let actions = crate::engine::legal_actions(&suspended, &catalog);
        assert_eq!(
            actions,
            vec![
                Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
                Action::Select {
                    answer: SelectionAnswer::OptionIndex(1),
                },
            ]
        );

        let resumed = apply_action(&suspended, &catalog, &actions[0])
            .unwrap()
            .state;
        assert!(resumed.pending.is_none());
        assert!(resumed.frames.is_empty());
        assert_eq!(resumed.block, 8, "resume did not replay the prefix");
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 1);
        assert_eq!(resumed.piles.get(PileId::Discard).len(), 2);
        assert_eq!(resumed.history.discarded_cards_this_turn, 1);
        assert!(
            !resumed.exact_piles,
            "an ordinary selected discard stays projection-neutral"
        );
    }

    #[test]
    fn public_actions_and_selection_keep_distinct_genetic_algorithm_tails() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::Survivor, CardId::GeneticAlgorithm] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let ga_atom = catalog.atom(&identity(CardId::GeneticAlgorithm)).unwrap();
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Survivor, 1),
            HotCard {
                uid: 2,
                atom: ga_atom,
                flags: physical | CARD_FLAG_RINGING,
            },
            HotCard {
                uid: 3,
                atom: ga_atom,
                flags: physical | CARD_FLAG_GENETIC_ALGORITHM_STATE,
            },
        ]);
        state.card_states.set(
            3,
            CardInstanceState {
                genetic_algorithm: GeneticAlgorithmState::from_parts(1, None).unwrap(),
                ..CardInstanceState::default()
            },
        );

        let initial = crate::engine::legal_actions(&state, &catalog);
        assert!(
            initial
                .iter()
                .any(|action| { matches!(action, Action::Play { uid: 2, .. }) })
        );
        assert!(
            initial
                .iter()
                .any(|action| { matches!(action, Action::Play { uid: 3, .. }) })
        );

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
        let actions = crate::engine::legal_actions(&suspended, &catalog);
        assert_eq!(actions.len(), 2, "both physical picks remain selectable");

        let first = apply_action(&suspended, &catalog, &actions[0])
            .unwrap()
            .state;
        let second = apply_action(&suspended, &catalog, &actions[1])
            .unwrap()
            .state;
        assert!(
            first
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 3),
            "reverse singleton expansion offers the GA tag after Ringing first"
        );
        assert!(
            second
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 2)
        );
    }

    #[test]
    fn an_exhausting_selector_projects_and_reloads_its_frozen_result_route() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::ThinkingAhead,
            CardId::StrikeSilent,
            CardId::DefendSilent,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::ThinkingAhead, 1),
            card(&catalog, CardId::StrikeSilent, 2),
            card(&catalog, CardId::DefendSilent, 3),
        ]);

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
        let canonical = HotBoundary::to_canonical(&suspended, &catalog);

        assert_eq!(
            canonical.continuations[0].fields["result_location"],
            serde_json::json!(["exhaust", "bottom"])
        );
        let reloaded_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let reloaded = HotBoundary::from_canonical(&canonical, &reloaded_catalog).unwrap();
        assert_eq!(
            HotBoundary::to_canonical(&reloaded, &reloaded_catalog),
            canonical
        );
    }

    #[test]
    fn burst_replay_suspends_each_body_and_never_replays_a_prefix() {
        let (mut state, catalog) = survivor_fixture();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::StrikeSilent, 4));
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);

        let first = apply_action(
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
        assert_eq!(first.block, 8);
        assert_eq!(pending_record(&first).play_index, 0);

        let second = apply_action(&first, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(second.block, 16, "the replay ran its own prefix once");
        assert_eq!(pending_record(&second).play_index, 1);
        assert_eq!(second.history.card_plays_finished_combat, 1);

        let finished = apply_action(&second, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert!(finished.pending.is_none());
        assert!(finished.frames.is_empty());
        assert_eq!(finished.block, 16);
        assert_eq!(finished.history.card_plays_finished_combat, 2);
        assert_eq!(finished.history.discarded_cards_this_turn, 2);
        assert_eq!(finished.powers.value(PowerId::Burst), 0);
    }

    #[test]
    fn targeted_headbutt_round_trips_target_then_moves_exact_discard_uid_to_draw_top() {
        let mut builder = CatalogBuilder::new();
        for id in [
            CardId::Headbutt,
            CardId::StrikeIronclad,
            CardId::DefendIronclad,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::Headbutt, 1));
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
        ]);

        let suspended = apply_action(
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
        assert_eq!(suspended.monsters[0].hp, 31);
        assert_eq!(
            pending_record(&suspended).target.map(|target| target.0),
            Some(0)
        );
        let canonical = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(canonical.continuations[0].fields["choice"], 0);
        assert_eq!(
            canonical.continuations[0].fields["target_identity"],
            serde_json::json!([0, 0])
        );
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(reloaded, suspended);

        let mut auto = canonical.clone();
        auto.continuations[0]
            .fields
            .insert("source".to_owned(), serde_json::json!("auto"));
        auto.continuations[0]
            .fields
            .insert("choice".to_owned(), serde_json::Value::Null);
        let auto_hot = HotBoundary::from_canonical(&auto, &catalog).unwrap();
        assert_eq!(
            pending_record(&auto_hot).source,
            crate::hot::CardPlaySource::Auto
        );
        assert_eq!(
            pending_record(&auto_hot).choice_kind,
            crate::hot::CardPlayChoiceKind::None
        );
        assert_eq!(HotBoundary::to_canonical(&auto_hot, &catalog), auto);

        let mut auto_with_manual_choice = auto.clone();
        auto_with_manual_choice.continuations[0]
            .fields
            .insert("choice".to_owned(), serde_json::json!(0));
        assert!(HotBoundary::from_canonical(&auto_with_manual_choice, &catalog).is_err());
        let mut manual_without_choice = canonical.clone();
        manual_without_choice.continuations[0]
            .fields
            .insert("choice".to_owned(), serde_json::Value::Null);
        assert!(HotBoundary::from_canonical(&manual_without_choice, &catalog).is_err());
        let mut redundant_unique_fallback = canonical.clone();
        redundant_unique_fallback.continuations[0]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([0, 0, 0]));
        assert!(HotBoundary::from_canonical(&redundant_unique_fallback, &catalog).is_err());

        let finished = apply_action(&reloaded, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert!(finished.pending.is_none());
        assert_eq!(finished.monsters[0].hp, 31, "attack prefix did not replay");
        assert_eq!(finished.piles.get(PileId::Draw).as_slice()[0].uid, 2);
        assert_eq!(finished.piles.get(PileId::Discard).len(), 2);
    }

    #[test]
    fn brand_exhaust_and_prepared_multi_pick_use_the_same_exact_consumer_gate() {
        let mut builder = CatalogBuilder::new();
        for identity in [
            identity(CardId::Brand),
            CardIdentity {
                id: CardId::Prepared,
                upgrade: 1,
                enchantment: None,
            },
            identity(CardId::StrikeSilent),
            identity(CardId::DefendSilent),
        ] {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();

        let mut brand = HotState::at_defaults();
        brand.hp = 70;
        brand.energy = 3;
        brand.next_card_uid = 4;
        brand.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Brand, 1),
            card(&catalog, CardId::StrikeSilent, 2),
            card(&catalog, CardId::DefendSilent, 3),
        ]);
        let brand = apply_action(
            &brand,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        for action in crate::engine::legal_actions(&brand, &catalog) {
            let Action::Select { answer } = action else {
                panic!("expected selection")
            };
            let before = brand.clone();
            let described = selected_card_uids(&brand, &catalog, answer)
                .unwrap()
                .unwrap();
            assert_eq!(brand, before, "description is read-only");
            let after = apply_action(&brand, &catalog, &action).unwrap().state;
            assert_eq!(
                described,
                after
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .map(|c| c.uid)
                    .collect::<Vec<_>>()
            );
        }
        let brand = apply_action(&brand, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(brand.history.owner_cards_exhausted_combat, 1);
        assert_eq!(brand.piles.get(PileId::Exhaust).len(), 1);

        let prepared_atom = catalog
            .atom(&CardIdentity {
                id: CardId::Prepared,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let mut prepared = HotState::at_defaults();
        prepared.hp = 70;
        prepared.energy = 3;
        prepared.next_card_uid = 14;
        prepared.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 10,
                atom: prepared_atom,
                flags: 0,
            },
            card(&catalog, CardId::StrikeSilent, 11),
            card(&catalog, CardId::DefendSilent, 12),
            card(&catalog, CardId::StrikeSilent, 13),
        ]);
        let prepared = apply_action(
            &prepared,
            &catalog,
            &Action::Play {
                uid: 10,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        // Three candidates choose two, in both pick orders (#2524).
        assert_eq!(crate::engine::legal_actions(&prepared, &catalog).len(), 6);
        let mut seen = std::collections::BTreeSet::new();
        for action in crate::engine::legal_actions(&prepared, &catalog) {
            let Action::Select { answer } = action else {
                panic!("expected selection")
            };
            let described = selected_card_uids(&prepared, &catalog, answer)
                .unwrap()
                .unwrap();
            assert_eq!(described.len(), 2);
            assert!(seen.insert(described.clone()), "each order is one action");
            let after = apply_action(&prepared, &catalog, &action).unwrap().state;
            let discarded = after.piles.get(PileId::Discard).as_slice();
            // The Discard pile records the picks in pick order.
            assert_eq!(
                discarded[..2].iter().map(|c| c.uid).collect::<Vec<_>>(),
                described
            );
            assert!(described.iter().all(|uid| {
                !after
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .any(|c| c.uid == *uid)
            }));
        }
        let prepared = apply_action(&prepared, &catalog, &select_action(0))
            .unwrap()
            .state;
        assert_eq!(prepared.history.discarded_cards_this_turn, 2);
        assert_eq!(prepared.piles.get(PileId::Hand).len(), 1);
    }

    #[test]
    fn payload_order_includes_physical_rows_and_equal_payloads_keep_pile_order() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        for (uid, amount) in [(2, -1), (1, 1), (3, 1)] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom: catalog.atom(&identity(CardId::DefendSilent)).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state.card_states.set(
                uid,
                CardInstanceState {
                    local_cost_modifiers: crate::hot::LocalCostModifiers::from_rows(vec![
                        LocalCostModifier {
                            kind: LocalCostModifierKind::Add,
                            amount,
                            expiration: LocalCostExpiration::ThisTurn,
                            reduce_only: false,
                        },
                    ]),
                    ..CardInstanceState::default()
                },
            );
        }
        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
    }

    #[test]
    fn ringing_tail_orders_after_an_equal_physical_payload_and_preserves_ties() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::DefendSilent)).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 3,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
            },
            HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_RINGING,
            },
        ]);

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 3, 2]
        );
    }

    #[test]
    fn genetic_algorithm_tails_follow_the_canonical_python_tuple_order() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::GeneticAlgorithm)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::GeneticAlgorithm)).unwrap();
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 5,
                atom,
                flags: physical | CARD_FLAG_GENETIC_ALGORITHM_STATE | CARD_FLAG_RINGING,
            },
            HotCard {
                uid: 4,
                atom,
                flags: physical | CARD_FLAG_GENETIC_ALGORITHM_STATE,
            },
            HotCard {
                uid: 3,
                atom,
                flags: physical | CARD_FLAG_GENETIC_ALGORITHM_STATE,
            },
            HotCard {
                uid: 2,
                atom,
                flags: physical | CARD_FLAG_RINGING,
            },
            HotCard {
                uid: 1,
                atom,
                flags: physical,
            },
        ]);
        for (uid, growth, row) in [(3, 1, Some(9)), (4, 2, None), (5, 2, None)] {
            state.card_states.set(
                uid,
                CardInstanceState {
                    genetic_algorithm: GeneticAlgorithmState::from_parts(growth, row).unwrap(),
                    ..CardInstanceState::default()
                },
            );
        }

        assert_eq!(
            payload_len(
                state.piles.get(PileId::Hand).as_slice()[1],
                identity(CardId::GeneticAlgorithm),
                &state.card_states.get(4)
            ),
            9
        );
        assert_eq!(
            payload_len(
                state.piles.get(PileId::Hand).as_slice()[0],
                identity(CardId::GeneticAlgorithm),
                &state.card_states.get(5)
            ),
            10
        );
        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5],
            "physical < Ringing tag < GA growth, then GA+Ringing length"
        );
    }

    #[test]
    fn equal_genetic_algorithm_rows_order_bound_before_hexed_before_ringing() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::GeneticAlgorithm)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::GeneticAlgorithm)).unwrap();
        let base = CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE;
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 3,
                atom,
                flags: base | CARD_FLAG_RINGING,
            },
            HotCard {
                uid: 1,
                atom,
                flags: base | CARD_FLAG_BOUND,
            },
            HotCard {
                uid: 2,
                atom,
                flags: base | CARD_FLAG_HEXED,
            },
        ]);
        for uid in 1..=3 {
            state.card_states.set(
                uid,
                CardInstanceState {
                    genetic_algorithm: GeneticAlgorithmState::from_parts(4, Some(7)).unwrap(),
                    ..CardInstanceState::default()
                },
            );
        }

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn sovereign_base_replay_orders_absent_before_explicit_zero_before_positive() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::SovereignBlade)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::SovereignBlade)).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 3,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
            HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
            HotCard {
                uid: 2,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
        ]);
        for (uid, replay) in [(2, 0), (3, 1)] {
            let mut instance = CardInstanceState::default();
            instance.set_base_replay_count(Some(replay)).unwrap();
            state.card_states.set(uid, instance);
        }

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn sovereign_damage_slot_precedes_physical_tuple_length() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::SovereignBlade)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::SovereignBlade)).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
            HotCard {
                uid: 2,
                atom,
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE | CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut high_damage = CardInstanceState {
            damage_growth: 20,
            ..CardInstanceState::default()
        };
        let mut low_damage = CardInstanceState {
            damage_growth: 10,
            ..CardInstanceState::default()
        };
        low_damage.set_base_replay_count(Some(0)).unwrap();
        high_damage.set_base_replay_count(None).unwrap();
        state.card_states.set(1, high_damage);
        state.card_states.set(2, low_damage);

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [2, 1]
        );
    }

    #[test]
    fn thrash_decimal_orders_at_the_native_physical_growth_position() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Thrash)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::Thrash)).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 2,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut fractional = CardInstanceState::default();
        fractional
            .set_fraction_damage_growth(crate::decimal::DotNetDecimal::ratio(21, 5).unwrap())
            .unwrap();
        let integer = CardInstanceState {
            damage_growth: 5,
            ..CardInstanceState::default()
        };
        state.card_states.set(1, fractional);
        state.card_states.set(2, integer);

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn equal_genetic_algorithm_growth_refuses_none_against_integer_deck_row() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::GeneticAlgorithm)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::GeneticAlgorithm)).unwrap();
        let flags = CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_GENETIC_ALGORITHM_STATE;
        let left = HotCard {
            uid: 1,
            atom,
            flags,
        };
        let right = HotCard {
            uid: 2,
            atom,
            flags,
        };
        let mut state = HotState::at_defaults();
        state.card_states.set(
            1,
            CardInstanceState {
                genetic_algorithm: GeneticAlgorithmState::from_parts(2, None).unwrap(),
                ..CardInstanceState::default()
            },
        );
        state.card_states.set(
            2,
            CardInstanceState {
                genetic_algorithm: GeneticAlgorithmState::from_parts(2, Some(1)).unwrap(),
                ..CardInstanceState::default()
            },
        );

        assert_eq!(
            card_payload_cmp(&state, &catalog, left, right),
            Err(EngineRefusal::MalformedArgs(
                "selection Genetic Algorithm deck-row order"
            ))
        );
    }

    #[test]
    fn local_keyword_payload_order_matches_python_sorted_tuples() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::DefendSilent)).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity(CardId::DefendSilent)).unwrap();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([4, 3, 2, 1].map(|uid| HotCard {
                uid,
                atom,
                flags: 0,
            }));
        state.card_states.set_local_sly(4);
        state.card_states.set_local_retain(3);
        state.card_states.set_local_sly(3);
        state.card_states.set_local_retain(2);

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();

        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn glam_used_wire_name_precedes_later_physical_payload_slots() {
        let identity = CardIdentity {
            id: CardId::DefendSilent,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity).unwrap();
        let mut state = HotState::at_defaults();
        let spent = HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let fresh_sly = HotCard {
            uid: 2,
            atom,
            flags: 0,
        };
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([spent, fresh_sly]);
        state.card_states.set_local_sly(2);
        let mut spent_state = state.card_states.get(1);
        spent_state.enchantment_state =
            crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
        state.card_states.set(1, spent_state);

        let ordered = sorted_exact_candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: None,
                operation: Operation::Discard,
            },
        )
        .unwrap();
        assert_eq!(
            ordered.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [2, 1],
            "GLAM sorts before GLAM_USED at canonical payload slot two, before local Sly"
        );
    }

    #[test]
    fn skill_without_sly_filter_excludes_native_and_local_keywords() {
        let mut builder = CatalogBuilder::new();
        for id in [CardId::DefendSilent, CardId::Reflex] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::Reflex, 1),
            card(&catalog, CardId::DefendSilent, 2),
            card(&catalog, CardId::DefendSilent, 3),
        ]);
        state.card_states.set_local_sly(3);

        let selected = candidates(
            &state,
            &catalog,
            Selector {
                pile: PileId::Hand,
                min: 1,
                max: 1,
                filter: Some(FilterMode::SkillWithoutSlyThisTurn),
                operation: Operation::Discard,
            },
        )
        .unwrap();

        assert_eq!(
            selected.iter().map(|card| card.uid).collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn generic_pending_round_trips_and_bad_answer_is_atomic() {
        let (mut state, catalog) = survivor_fixture();
        state.powers.set(PowerId::Rupture, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
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
        let canonical = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            canonical.continuations[0].fields["latch"][4], 1,
            "BeforeCardPlayed Rupture registration is frozen across suspension"
        );
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(reloaded, suspended);

        let mut wrong_remainder = canonical.clone();
        wrong_remainder.continuations[0].fields.insert(
            "remaining_steps".to_owned(),
            serde_json::json!([["block", 8]]),
        );
        assert!(HotBoundary::from_canonical(&wrong_remainder, &catalog).is_err());
        let mut wrong_target = canonical.clone();
        wrong_target.continuations[0]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([0, 0]));
        assert!(HotBoundary::from_canonical(&wrong_target, &catalog).is_err());
        let mut wrong_selector = canonical.clone();
        wrong_selector.player.insert(
            "pending".to_owned(),
            serde_json::json!(["frame_select", 1, ["select", "hand", 2, 2, null, "discard"]]),
        );
        assert!(HotBoundary::from_canonical(&wrong_selector, &catalog).is_err());

        let before = reloaded.clone();
        assert!(
            apply_action(
                &reloaded,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(9),
                },
            )
            .is_err()
        );
        assert_eq!(reloaded, before);
    }

    #[test]
    fn one_candidate_auto_resolves_without_exposing_a_choice() {
        let (mut state, catalog) = survivor_fixture();
        state.piles.get_mut(PileId::Hand).make_mut().pop();
        let finished = apply_action(
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
        assert!(finished.pending.is_none());
        assert_eq!(finished.history.discarded_cards_this_turn, 1);
        assert_eq!(finished.piles.get(PileId::Hand).len(), 0);
        assert_eq!(finished.piles.get(PileId::Discard).len(), 2);
    }

    #[test]
    fn glimmer_suffix_suspends_only_for_two_live_cards_and_auto_moves_one() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();

        let mut one = HotState::at_defaults();
        one.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, CardId::StrikeIronclad, 1));
        assert_eq!(
            execute_glimmer(&mut one, &catalog, &mut Vec::new()).unwrap(),
            SelectDisposition::Complete
        );
        assert!(one.piles.get(PileId::Hand).is_empty());
        assert_eq!(one.piles.get(PileId::Draw).as_slice()[0].uid, 1);

        let mut two = HotState::at_defaults();
        two.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::StrikeIronclad, 3),
        ]);
        let before = two.clone();
        assert_eq!(
            execute_glimmer(&mut two, &catalog, &mut Vec::new()).unwrap(),
            SelectDisposition::Suspend
        );
        assert_eq!(two, before, "suspension publishes no partial suffix");

        two.history.over = true;
        let terminal = two.clone();
        assert_eq!(
            execute_glimmer(&mut two, &catalog, &mut Vec::new()).unwrap(),
            SelectDisposition::Complete
        );
        assert_eq!(two, terminal, "terminal selection is an exact no-op");
    }

    /// Under Whispering Earring's `VakuuCardSelector` (#3414) Glimmer's
    /// put-back takes Hand's first card (`VakuuCardSelector::GetSelectedCards`
    /// RVA `0x9d733`, `options.Take(maxSelect)`); leaving the scope restores
    /// the player's choice.
    #[test]
    fn vakuu_scope_puts_back_the_first_hand_card_for_glimmer() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::StrikeIronclad, 3),
        ]);
        let before = state.clone();
        {
            let _vakuu = VakuuSelectorScope::enter();
            assert_eq!(
                execute_glimmer(&mut state, &catalog, &mut Vec::new()).unwrap(),
                SelectDisposition::Complete
            );
        }
        let uids = |pile| -> Vec<u32> {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect()
        };
        assert_eq!(uids(PileId::Hand), [3]);
        assert_eq!(uids(PileId::Draw), [2]);

        let mut outside = before.clone();
        assert_eq!(
            execute_glimmer(&mut outside, &catalog, &mut Vec::new()).unwrap(),
            SelectDisposition::Suspend
        );
        assert_eq!(outside, before);
    }

    fn cosmic_indifference_style_fixture(id: CardId) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        for id in [id, CardId::StrikeIronclad, CardId::DefendIronclad] {
            builder.intern(identity(id)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(&catalog, id, 1));
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::DefendIronclad, 3),
        ]);
        (state, catalog)
    }

    /// Cosmic Indifference under the scope takes the Discard pile's first
    /// card to Draw's top (`vakuu_program_is_exact`); Headbutt, the same
    /// Discard-to-Draw-top shape but a `CardSelectCmd` path not read for
    /// #3414, still suspends for the player.
    #[test]
    fn vakuu_scope_resolves_only_cosmic_indifference_among_discard_pickers() {
        let (state, catalog) = cosmic_indifference_style_fixture(CardId::CosmicIndifference);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let resolved = {
            let _vakuu = VakuuSelectorScope::enter();
            apply_action(&state, &catalog, &play).unwrap().state
        };
        assert!(resolved.pending.is_none());
        assert_eq!(resolved.piles.get(PileId::Draw).as_slice()[0].uid, 2);
        let outside = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(
            outside.pending.is_some(),
            "the player chooses without the scope"
        );

        let (state, catalog) = cosmic_indifference_style_fixture(CardId::Headbutt);
        let headbutt = Action::Play {
            uid: 1,
            target: Some(0),
            selection: SelectionRef::NONE,
        };
        let _vakuu = VakuuSelectorScope::enter();
        let parked = apply_action(&state, &catalog, &headbutt).unwrap().state;
        assert!(
            parked.pending.is_some(),
            "an unread selector is never auto-picked"
        );
    }

    #[test]
    fn glimmer_pending_actions_match_the_reusable_buffer_api() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::Glimmer)).unwrap();
        builder.intern(identity(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        let active = card(&catalog, CardId::Glimmer, 1);
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Play).make_mut().push(active);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            card(&catalog, CardId::StrikeIronclad, 2),
            card(&catalog, CardId::StrikeIronclad, 3),
        ]);
        state.next_card_uid = 4;
        let mut record = crate::hot::CardPlayRecord::pending_for_test(active.uid);
        record.selection_kind = Some(PendingSelectionKind::Glimmer);
        record.spent = 1;
        let pending = state.frames.push_pending_for_test(&record);
        state.pending = Some(std::sync::Arc::new(pending));

        assert_eq!(
            special_option_count(&state, &catalog, state.pending.as_deref().unwrap()),
            Ok(2)
        );
        assert_buffered_actions_match(&state, &catalog);
    }

    /// #2985: Headbutt's Discard pick over a Hexed Stoke+ enchanted with
    /// Steady and its bare Hexed twin. Both payloads run to the affliction
    /// tail, so Python compares the enchantment slot `Some` against `None`
    /// and raises. Where the candidate order only labels the answers, the
    /// bare twin sorts first; where it reaches state, the refusal stands.
    #[test]
    fn issue2985_label_only_order_puts_the_bare_twin_first_and_python_order_refuses() {
        use crate::hot::CARD_FLAG_HEXED;
        let enchanted_identity = CardIdentity {
            id: CardId::Stoke,
            upgrade: 1,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Steady,
                amount: 1,
            }),
        };
        let plain_identity = CardIdentity {
            enchantment: None,
            ..enchanted_identity
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(enchanted_identity).unwrap();
        builder.intern(plain_identity).unwrap();
        let catalog = builder.build();
        let flags = CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED;
        let enchanted = HotCard {
            uid: 3,
            atom: catalog.atom(&enchanted_identity).unwrap(),
            flags,
        };
        let plain = HotCard {
            uid: 42,
            atom: catalog.atom(&plain_identity).unwrap(),
            flags,
        };
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend([enchanted, plain]);
        state.next_card_uid = 43;
        let refusal = EngineRefusal::MalformedArgs("selection enchantment payload order");

        for (left, right) in [(enchanted, plain), (plain, enchanted)] {
            assert_eq!(
                card_payload_cmp(&state, &catalog, left, right),
                Err(refusal.clone())
            );
        }
        assert_eq!(
            card_payload_cmp_in(&state, &catalog, plain, enchanted, PayloadOrder::LabelOnly),
            Ok(Ordering::Less)
        );
        assert_eq!(
            card_payload_cmp_in(&state, &catalog, enchanted, plain, PayloadOrder::LabelOnly),
            Ok(Ordering::Greater)
        );

        let selector = |max, operation| Selector {
            pile: PileId::Discard,
            min: max,
            max,
            filter: None,
            operation,
        };
        let headbutt = selector(
            1,
            Operation::Move {
                destination: PileId::Draw,
                top: true,
            },
        );
        let ordered_move = selector(
            2,
            Operation::Move {
                destination: PileId::Draw,
                top: true,
            },
        );
        let single_discard = selector(1, Operation::Discard);
        let double_discard = selector(2, Operation::Discard);
        let double_upgrade = selector(2, Operation::Upgrade);
        for (selector, expected) in [
            (headbutt, PayloadOrder::LabelOnly),
            (single_discard, PayloadOrder::LabelOnly),
            (ordered_move, PayloadOrder::LabelOnly),
            (double_discard, PayloadOrder::LabelOnly),
            (double_upgrade, PayloadOrder::PythonTuple),
        ] {
            assert_eq!(candidate_order_is_label_only(selector), expected);
        }
        for selector in [headbutt, single_discard, ordered_move, double_discard] {
            assert_eq!(
                sorted_exact_candidates(&state, &catalog, selector)
                    .unwrap()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [42, 3]
            );
        }
        // Headbutt offers one answer per physical card; the pick-order fan-out
        // offers both orders of the pair. Neither set depends on the sort.
        assert_eq!(options(&state, &catalog, headbutt).unwrap().len(), 2);
        assert_eq!(options(&state, &catalog, ordered_move).unwrap().len(), 2);
        // A two-card Discard now fans out both orders too (#2524).
        assert_eq!(options(&state, &catalog, double_discard).unwrap().len(), 2);
        // An order-unobservable two-card sink would apply the sorted order,
        // so it keeps Python's refusal.
        assert_eq!(
            sorted_exact_candidates(&state, &catalog, double_upgrade),
            Err(refusal.clone())
        );
        assert_eq!(options(&state, &catalog, double_upgrade), Err(refusal));
    }

    /// #3075 selection Discard: the picks stay in their pile until
    /// DiscardAndDraw moves them, in pick order (#3102); once combat is over
    /// the whole command is a no-op and nothing leaves Hand.
    #[test]
    fn selection_discard_moves_in_pick_order_and_is_a_noop_once_combat_is_over() {
        let mut builder = CatalogBuilder::new();
        builder.intern(identity(CardId::StrikeSilent)).unwrap();
        let catalog = builder.build();
        let picked = [
            card(&catalog, CardId::StrikeSilent, 2),
            card(&catalog, CardId::StrikeSilent, 1),
        ];
        let selector = Selector {
            pile: PileId::Hand,
            min: 2,
            max: 2,
            filter: None,
            operation: Operation::Discard,
        };
        for over in [false, true] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.piles.get_mut(PileId::Hand).make_mut().extend([
                card(&catalog, CardId::StrikeSilent, 1),
                card(&catalog, CardId::StrikeSilent, 2),
                card(&catalog, CardId::StrikeSilent, 3),
            ]);
            state.history.over = over;
            let before = state.clone();
            let mut events = Vec::new();

            apply(&mut state, &catalog, selector, &picked, &mut events).unwrap();

            if over {
                assert_eq!(state, before);
                assert!(events.is_empty());
            } else {
                let uids = |pile| {
                    state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .map(|card: &HotCard| card.uid)
                        .collect::<Vec<_>>()
                };
                assert_eq!(uids(PileId::Discard), [2, 1]);
                assert_eq!(uids(PileId::Hand), [3]);
                assert_eq!(state.history.discarded_cards_this_turn, 2);
            }
        }
    }

    fn transfigure_args() -> [CompiledArg; 5] {
        [
            CompiledArg::Pile(PileId::Hand),
            CompiledArg::I(1),
            CompiledArg::I(1),
            CompiledArg::Nil,
            CompiledArg::Word(StepWord::TransfigureExact),
        ]
    }

    fn transfigure_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        for upgrade in 0..=1 {
            builder
                .intern(CardIdentity {
                    id: CardId::Transfigure,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
        }
        for id in [
            CardId::StrikeIronclad,
            CardId::Whirlwind,
            CardId::Dazed,
            CardId::SculptingStrike,
        ] {
            builder.intern(identity(id)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.build()
    }

    fn transfigure_row() -> LocalCostModifier {
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: 1,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        }
    }

    /// #3291: the writer word binds only to Transfigure's exact L0/L1 rows,
    /// exact-one, unfiltered, from Hand.
    #[test]
    fn transfigure_selector_is_bound_to_its_exact_rows() {
        let catalog = transfigure_catalog();
        for upgrade in 0..=1 {
            let owner = catalog
                .spec(
                    catalog
                        .atom(&CardIdentity {
                            id: CardId::Transfigure,
                            upgrade,
                            enchantment: None,
                        })
                        .unwrap(),
                )
                .unwrap();
            let selector = selector_from_args(&catalog, owner, &transfigure_args()).unwrap();
            assert_eq!(selector.operation, Operation::Transfigure);
            assert!(!selection_order_is_observable(selector.operation));
            let mut two = transfigure_args();
            two[2] = CompiledArg::I(2);
            assert_eq!(
                selector_from_args(&catalog, owner, &two).unwrap_err(),
                EngineRefusal::MalformedArgs("select operation")
            );
            let mut filtered = transfigure_args();
            filtered[3] = CompiledArg::Filter(FilterMode::Upgradable);
            assert_eq!(
                selector_from_args(&catalog, owner, &filtered).unwrap_err(),
                EngineRefusal::MalformedArgs("select operation")
            );
            let mut draw = transfigure_args();
            draw[0] = CompiledArg::Pile(PileId::Draw);
            assert_eq!(
                selector_from_args(&catalog, owner, &draw).unwrap_err(),
                EngineRefusal::MalformedArgs("select operation")
            );
        }
        let forged = catalog
            .spec(catalog.atom(&identity(CardId::SculptingStrike)).unwrap())
            .unwrap();
        assert_eq!(
            selector_from_args(&catalog, forged, &transfigure_args()).unwrap_err(),
            EngineRefusal::MalformedArgs("select operation")
        );
    }

    /// #3291: every branch of `apply_transfigure` — the cost write and its
    /// X-cost / negative-base skips, the unconditional replay increment,
    /// stacking, the empty answer, and the three named refusals.
    #[test]
    fn transfigure_writes_cost_then_replay_on_the_exact_selected_card() {
        let catalog = transfigure_catalog();
        let owner = catalog
            .spec(catalog.atom(&identity(CardId::Transfigure)).unwrap())
            .unwrap();
        let selector = selector_from_args(&catalog, owner, &transfigure_args()).unwrap();
        let strike = card(&catalog, CardId::StrikeIronclad, 1);
        let whirlwind = card(&catalog, CardId::Whirlwind, 2);
        let dazed = card(&catalog, CardId::Dazed, 3);
        let twin = card(&catalog, CardId::StrikeIronclad, 4);
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([strike, whirlwind, dazed, twin]);
        let strike_spec = catalog.spec(strike.atom).unwrap();

        // Ordinary cost: Add(+1, ThisCombat) and one replay, in place.
        let mut one = state.clone();
        apply(&mut one, &catalog, selector, &[strike], &mut Vec::new()).unwrap();
        let instance = one.card_states.get(strike.uid);
        assert_eq!(
            instance.local_cost_modifiers.as_slice(),
            &[transfigure_row()]
        );
        assert_eq!(instance.base_replay_count(), Some(1));
        let hand = one.piles.get(PileId::Hand).as_slice();
        assert_eq!(
            hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "the selected card keeps its Hand position"
        );
        assert_ne!(hand[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(hand[3].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert!(one.exact_piles);
        assert_eq!(
            crate::engine::play::resolved_energy_cost(&one, hand[0], strike_spec),
            2
        );
        assert_eq!(one.card_states.get(twin.uid), CardInstanceState::default());

        // A second Transfigure stacks both writes.
        let mut stacked = one.clone();
        let live = stacked.piles.get(PileId::Hand).as_slice()[0];
        apply(&mut stacked, &catalog, selector, &[live], &mut Vec::new()).unwrap();
        let instance = stacked.card_states.get(strike.uid);
        assert_eq!(
            instance.local_cost_modifiers.as_slice(),
            &[transfigure_row(), transfigure_row()]
        );
        assert_eq!(instance.base_replay_count(), Some(2));
        assert_eq!(
            crate::engine::play::resolved_energy_cost(&stacked, live, strike_spec),
            3
        );

        // X-cost (IL_0141) and negative base (IL_0151) skip only the cost.
        for skipped in [whirlwind, dazed] {
            let mut next = state.clone();
            apply(&mut next, &catalog, selector, &[skipped], &mut Vec::new()).unwrap();
            let instance = next.card_states.get(skipped.uid);
            assert!(instance.local_cost_modifiers.is_empty());
            assert_eq!(instance.base_replay_count(), Some(1));
            assert!(next.exact_piles);
        }

        // An empty Hand answer is a no-op.
        let mut empty = state.clone();
        apply(&mut empty, &catalog, selector, &[], &mut Vec::new()).unwrap();
        assert_eq!(empty, state);

        // Named refusals: plural answer, legacy identity, vanished card.
        assert_eq!(
            apply(
                &mut state.clone(),
                &catalog,
                selector,
                &[strike, twin],
                &mut Vec::new()
            ),
            Err(EngineRefusal::MalformedArgs(
                "Transfigure exact-one selection"
            ))
        );
        let legacy = legacy_card(&catalog, CardId::StrikeIronclad);
        let mut with_legacy = state.clone();
        with_legacy
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(legacy);
        assert_eq!(
            apply(
                &mut with_legacy,
                &catalog,
                selector,
                &[legacy],
                &mut Vec::new()
            ),
            Err(EngineRefusal::MalformedArgs(
                "Transfigure selected physical identity"
            ))
        );
        let gone = card(&catalog, CardId::StrikeIronclad, 9);
        assert_eq!(
            apply(
                &mut state.clone(),
                &catalog,
                selector,
                &[gone],
                &mut Vec::new()
            ),
            Err(EngineRefusal::FrozenCardVanished {
                uid: 9,
                pile: PileId::Hand
            })
        );
    }

    /// #3291 end to end: Transfigure suspends on a two-card Hand, both the
    /// pending and the resolved state survive the canonical boundary, L0
    /// exhausts itself, and the selected Strike then costs 2 and plays twice.
    #[test]
    fn transfigure_suspends_round_trips_and_replays_the_selected_card() {
        let catalog = transfigure_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        let source = card(&catalog, CardId::Transfigure, 1);
        let first = card(&catalog, CardId::StrikeIronclad, 2);
        let second = card(&catalog, CardId::StrikeIronclad, 3);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, first, second]);

        let suspended = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(suspended.energy, 2);
        assert_eq!(
            pending_record(&suspended).route().unwrap().1,
            PendingSelectionKind::Program
        );
        assert_buffered_actions_match(&suspended, &catalog);
        let actions = legal_actions(&suspended, &catalog);
        assert_eq!(actions, [select_action(0), select_action(1)]);
        let canonical = HotBoundary::to_canonical(&suspended, &catalog);
        assert_eq!(
            canonical.player["pending"][2],
            serde_json::json!(["select", "hand", 1, 1, null, "transfigure_exact"])
        );
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(reloaded, suspended);

        let resolved = apply_action(&reloaded, &catalog, &actions[0])
            .unwrap()
            .state;
        assert!(resolved.pending.is_none());
        assert_eq!(resolved.piles.get(PileId::Exhaust).as_slice(), &[source]);
        let hand = resolved.piles.get(PileId::Hand).as_slice().to_vec();
        assert_eq!(hand.iter().map(|card| card.uid).collect::<Vec<_>>(), [2, 3]);
        let written: Vec<_> = hand
            .iter()
            .filter(|card| resolved.card_states.get(card.uid).base_replay_count() == Some(1))
            .copied()
            .collect();
        assert_eq!(written.len(), 1, "exactly one card is transfigured");
        let target = written[0];
        let canonical = HotBoundary::to_canonical(&resolved, &catalog);
        let reloaded = HotBoundary::from_canonical(&canonical, &catalog).unwrap();
        assert_eq!(
            reloaded, resolved,
            "cost row and replay survive the boundary"
        );

        let played = apply_action(
            &reloaded,
            &catalog,
            &Action::Play {
                uid: target.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(played.energy, 0, "the transfigured Strike costs 1 + 1");
        assert_eq!(played.monsters[0].hp, 40 - 6 - 6, "and plays twice");
    }

    /// #3291: an empty Hand and a one-card Hand complete without a pending
    /// screen (`candidates.len() <= min`).
    #[test]
    fn transfigure_empty_and_singleton_hands_complete_without_pending() {
        let catalog = transfigure_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 40));
        let source = leveled_card(&catalog, CardId::Transfigure, 1, 1);
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        let play = Action::Play {
            uid: source.uid,
            target: None,
            selection: SelectionRef::NONE,
        };

        let empty = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(empty.pending.is_none());
        assert_eq!(
            empty.piles.get(PileId::Discard).as_slice(),
            &[source],
            "Transfigure+ has no Exhaust"
        );
        assert!(empty.card_states.as_slice().is_empty());

        let strike = card(&catalog, CardId::StrikeIronclad, 2);
        let mut single = state.clone();
        single.piles.get_mut(PileId::Hand).make_mut().push(strike);
        let single = apply_action(&single, &catalog, &play).unwrap().state;
        assert!(single.pending.is_none());
        let instance = single.card_states.get(strike.uid);
        assert_eq!(instance.base_replay_count(), Some(1));
        assert_eq!(
            instance.local_cost_modifiers.as_slice(),
            &[transfigure_row()]
        );
    }

    /// `VisualCardPool.IsColorless` by pool: Colorless, Event, Token and
    /// Deprecated members are colorless unless a character visual pool
    /// overrides; character, Curse, Quest and Status cards are not.
    #[test]
    fn visual_card_pool_colorlessness_follows_the_native_pools() {
        use crate::ids::CardId;
        for id in [
            CardId::ThinkingAhead,
            CardId::Alchemize,
            CardId::Apotheosis,
            CardId::Shiv,
            CardId::DeprecatedCard,
        ] {
            assert!(super::visual_card_pool_is_colorless(id), "{id:?}");
        }
        for id in [
            CardId::Clash,
            CardId::Distraction,
            CardId::HelloWorld,
            CardId::StrikeIronclad,
            CardId::Burn,
            CardId::Regret,
            CardId::SpoilsMap,
        ] {
            assert!(!super::visual_card_pool_is_colorless(id), "{id:?}");
        }
    }
}
