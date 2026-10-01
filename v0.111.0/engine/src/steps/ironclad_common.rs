//! Card-step bodies for the `content/cards/ironclad_common.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1324/#1679/#1751): 7 of 7 ported
//!
//! Anger's attack-then-clone transaction is expressible with the exact live
//! physical source and generated-card primitives, and #1679 added Havoc's
//! forced-Exhaust draw-top AutoPlay route. The shared physical-upgrade
//! transaction and complete catalog/admission closure now make Armaments+'s
//! frozen Hand walk and Armaments L0's manual one-card Upgrade selection
//! total. Selecting direct-AutoPlay children remain capability-dark.

use super::StepCtx;
use crate::catalog::{CardSpec, Catalog, CompiledArg};
use crate::engine::EngineRefusal;
use crate::engine::cards::{inject_generated_clones_bottom, upgrade_live_cards_once};
use crate::engine::damage::{
    alive_targets, apply_card_monster_debuff_with_catalog, player_attack_all_from_card,
    player_attack_from_card,
};
use crate::engine::draw::detach_card_for_exhaust;
use crate::hot::{
    CARD_FLAG_LEGACY, HotCard, HotState, MiseryToken, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, PowerId, StepKind};
use crate::rng::Xoshiro256StarStar;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::AngerExact,
    StepKind::AttackPerStrike,
    StepKind::ExhaustRandom,
    StepKind::Havoc,
    StepKind::MoltenVuln,
    StepKind::ThunderclapExact,
    StepKind::UpgradeAllHand,
];

fn two_ints(ctx: &StepCtx<'_>, site: &'static str) -> Result<(i64, i64), EngineRefusal> {
    match ctx.args {
        [CompiledArg::I(first), CompiledArg::I(second)] => Ok((*first, *second)),
        _ => Err(EngineRefusal::MalformedArgs(site)),
    }
}

fn exact_active_anger(
    ctx: &StepCtx<'_>,
    dampen_restored_atom: Option<crate::catalog::CardAtom>,
) -> Result<HotCard, EngineRefusal> {
    let mut sources = PileId::ALL.into_iter().flat_map(|pile| {
        ctx.state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .copied()
            .filter(|card| card.uid == ctx.source_uid)
    });
    let Some(source) = sources.next() else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 0,
        });
    };
    let additional = sources.count();
    if additional != 0 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: ctx.source_uid,
            matches: 1 + additional,
        });
    }
    let identity = ctx
        .catalog
        .spec(source.atom)
        .ok_or(EngineRefusal::UnknownAtom(source.atom))?
        .identity;
    let restored_by_dampen = dampen_restored_atom == Some(source.atom)
        && ctx.state.card_states.dampen().is_none()
        && ctx.spec.identity.upgrade == 0
        && identity.upgrade == 1
        && identity.enchantment == ctx.spec.identity.enchantment;
    if (identity != ctx.spec.identity && !restored_by_dampen)
        || identity.id != CardId::Anger
        || !matches!(identity.upgrade, 0 | 1)
    {
        return Err(EngineRefusal::MalformedArgs("anger_exact"));
    }
    Ok(source)
}

/// The atom Dampen will restore this Anger to if its caster dies mid-play.
///
/// Only a row Dampen tracks for this exact physical UID qualifies, and the
/// post-attack re-resolve accepts it only once that row has actually been
/// consumed (the whole Dampen state is gone). See [`anger_exact`].
fn anger_dampen_restore_atom(ctx: &StepCtx<'_>) -> Option<crate::catalog::CardAtom> {
    ctx.state.card_states.dampen().and_then(|dampen| {
        dampen
            .cards
            .iter()
            .find(|row| row.uid == ctx.source_uid && row.old_level == 1)
            .map(|row| row.old_atom)
    })
}

/// Anger's targeted hit followed by an exact live physical clone.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) attacks first, re-resolves the
/// same physical source uid after the awaited attack, then
/// `add_generated_card_clones` records one owner-generated card
/// and inserts its exact payload at Discard/Bottom. The generation record is
/// before the combat-ending gate: a lethal hit therefore advances both
/// generation epochs without allocating or inserting a clone.
///
/// **Dampen restored mid-play (#2974).** Current-build authority is v0.111.0
/// `sts2.dll` SHA-256 `9cb4f1ad…fbf12b4`. `Anger/<OnPlay>d__3::MoveNext` (RVA
/// `0x389cd8`) awaits `AttackCommand.Execute` (IL_0028-IL_007f), then clones
/// `this` with `CardModel::CreateClone` (IL_00de, RVA `0x7e1e0`: `CardScope.
/// CloneCard(this)` of the *current* model, IL_0037-IL_0043) and adds it with
/// `CardPileCmd.AddGeneratedCardToCombat` (IL_00e5-IL_00ed). If that hit kills
/// Magi Knight, the last Dampen caster, `DampenPower/<AfterDeath>d__7::MoveNext`
/// (RVA `0x33871c`, IL_004c-IL_006c) removes the power inside the awaited
/// attack, and `DampenPower::AfterRemoved` (RVA `0xa1290`, IL_000c-IL_004d)
/// runs `CardCmd.Upgrade` on every downgraded card, this Anger included. The
/// clone therefore copies the restored L1 card. The re-resolve accepts exactly
/// that transition: the live atom is the row's recorded `old_atom`, the row
/// was for this UID at old level 1, the played spec was the dampened L0, and
/// Dampen is gone. Any other identity drift still refuses.
pub(crate) fn anger_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let damage = match (ctx.spec.identity.id, ctx.spec.identity.upgrade, ctx.args) {
        (CardId::Anger, 0, [CompiledArg::I(6)]) => 6,
        (CardId::Anger, 1, [CompiledArg::I(8)]) => 8,
        _ => return Err(EngineRefusal::MalformedArgs("anger_exact")),
    };
    exact_active_anger(ctx, None)?;
    let dampen_restored_atom = anger_dampen_restore_atom(ctx);
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
    let source = exact_active_anger(ctx, dampen_restored_atom)?;
    inject_generated_clones_bottom(
        ctx.state,
        ctx.catalog,
        source,
        1,
        PileId::Discard,
        ctx.events,
    )?;
    Ok(())
}

/// Perfected Strike's live strike-count scaling.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) reads the combat-wide Strike-card
/// census and issues one targeted player attack with `base + per * count`.
pub(crate) fn attack_per_strike(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (base, per_strike) = two_ints(ctx, "attack_per_strike")?;
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    let damage = base
        .checked_add(
            per_strike
                .checked_mul(i64::from(ctx.state.ps_strikes))
                .ok_or(EngineRefusal::CounterOverflow("attack_per_strike"))?,
        )
        .ok_or(EngineRefusal::CounterOverflow("attack_per_strike"))?;
    player_attack_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        &[target],
        damage,
        1,
        ctx.events,
    )
}

/// True Grit and Cinder's exact Selection-stream random Hand exhaust.
///
/// v0.111.0 DLL 9cb4f1ad: `Cinder/<OnPlay>d__5::MoveNext` (RVA 0x392350)
/// IL_011b–0143 and `TrueGrit/<OnPlay>d__7::MoveNext` (RVA 0x3c4950)
/// IL_01af–01d9 take `GetPile(2 = Hand)` and call
/// `CombatCardSelection.NextItem`, which consumes one bounded Selection draw
/// for a nonempty Hand whether or not combat has ended; the chosen card then
/// goes to `CardCmd.Exhaust` (Cinder IL_0155, True Grit IL_01ee). Exhaust's
/// state machine (RVA 0x3e06c8, IL_0020–0034) returns on `IsOverOrEnding`
/// before its `CardPileCmd.Add` (IL_0072), so after a killing blow the draw
/// is spent but the card stays in Hand (#3041).
pub(crate) fn exhaust_random(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("exhaust_random"));
    }
    let hand_len = ctx.state.piles.get(PileId::Hand).len();
    if hand_len == 0 {
        return Ok(());
    }
    let live = ctx.state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound: i32 = hand_len
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("exhaust_random hand"))?;
    let index: usize = rng
        .next_bounded(bound)
        .map_err(|_| EngineRefusal::CounterOverflow("exhaust_random hand"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("exhaust_random index"))?;
    ctx.state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    // The pick above always consumed its Selection draw; CardCmd.Exhaust is
    // gated on IsOverOrEnding, so after a killing blow the card stays in Hand.
    let Some(removed) = detach_card_for_exhaust(ctx.state, PileId::Hand, index)? else {
        return Ok(());
    };
    let _ = crate::engine::draw::card_exhausted_cardplay_return_only(
        ctx.state,
        ctx.catalog,
        removed,
        ctx.source_uid,
        ctx.events,
    )?;
    Ok(())
}

/// `havoc` — force-Exhaust the exact top Draw AutoPlay.
///
/// Python: `_havoc_flip` (frozen, deleted #2827) performs one atomic draw-top AutoPlay:
/// it normally reshuffles an empty Draw pile, moves the frozen top card to
/// Play, exhausts an unplayable card without play hooks or target RNG, and
/// otherwise autoplays without Energy spend while forcing its result to
/// Exhaust. The family-callable forced-result wrapper shares Mayhem's complete
/// gather/reshuffle/drain path and changes only that frozen result flag.
pub(crate) fn havoc(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.spec.identity.id != CardId::Havoc
        || !matches!(ctx.spec.identity.upgrade, 0 | 1)
    {
        return Err(EngineRefusal::MalformedArgs("havoc"));
    }
    crate::engine::play::autoplay_draw_top_restricted_parent(
        ctx.state,
        ctx.catalog,
        1,
        true,
        ctx.events,
    )
}

/// Molten Fist's live post-attack Vulnerable doubling.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) rechecks that the target survived
/// the preceding attack, reads its then-live Vulnerable amount, and applies
/// that exact amount again only when positive.
pub(crate) fn molten_vuln(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("molten_vuln"));
    }
    let target = ctx
        .target
        .ok_or(EngineRefusal::TargetMismatch { required: true })?;
    if !crate::engine::play::active_card_target_is_current(ctx.state, ctx.source_uid, target) {
        return Ok(());
    }
    if ctx.state.monsters[target].hp <= 0 {
        return Ok(());
    }
    let stacks = ctx.state.monsters[target].powers.value(PowerId::Vuln);
    if stacks <= 0 {
        return Ok(());
    }
    apply_card_monster_debuff_with_catalog(
        ctx.state,
        ctx.catalog,
        target,
        PowerId::Vuln,
        MiseryToken::Vuln,
        stacks,
        ctx.events,
    )
}

/// Thunderclap's all-enemy attack followed by live-survivor Vulnerable.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) snapshots the living targets for
/// the attack, then walks the post-attack living roster for the debuff while
/// respecting combat termination between effects.
pub(crate) fn thunderclap_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let (damage, vulnerable) = two_ints(ctx, "thunderclap_exact")?;
    let vulnerable: i32 = vulnerable
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("thunderclap_exact"))?;
    player_attack_all_from_card(
        ctx.state,
        (ctx.catalog, ctx.spec, ctx.source_uid),
        damage,
        1,
        ctx.events,
    )?;
    for target in alive_targets(ctx.state) {
        if ctx.state.history.over {
            break;
        }
        apply_card_monster_debuff_with_catalog(
            ctx.state,
            ctx.catalog,
            target,
            PowerId::Vuln,
            MiseryToken::Vuln,
            vulnerable,
            ctx.events,
        )?;
    }
    Ok(())
}

/// Armaments+'s exact frozen-Hand upgrade walk.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) snapshots Hand order, resolves
/// every card's exact next upgrade through `MAX_UPGRADE`, validates the full
/// batch, then atomically rewrites each physical card while preserving uid and
/// mutable instance state. Current-build `Armaments/<OnPlay>d__5::MoveNext`
/// RVA `0x38a310` performs the same ordered `IsUpgradable` walk after the
/// awaited Block. The boundary closes every cataloged next rung whenever a
/// non-Exhaust source is reachable, and the shared helper owns terminal
/// gating, complete preflight, local-cost clamping, and exact-pile promotion.
fn upgrade_all_hand_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    if spec.identity.id != CardId::Armaments
        || spec.identity.upgrade != 1
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return false;
    }
    let [block, upgrade] = catalog.steps(spec) else {
        return false;
    };
    block.kind == StepKind::Block
        && catalog.args(block.args) == [CompiledArg::I(5)]
        && upgrade.kind == StepKind::UpgradeAllHand
        && catalog.args(upgrade.args).is_empty()
}

/// Whether this is the exact current-build Armaments L0 one-card selector.
///
/// The selector is deliberately owner-bound: `SelectOp::Upgrade` is an open
/// consumer vocabulary elsewhere, while Armaments fixes Hand, one card,
/// `upgradable`, and one native Upgrade pass.
pub(crate) fn upgrade_one_hand_program_is_exact(catalog: &Catalog, spec: &CardSpec) -> bool {
    if spec.identity.id != CardId::Armaments
        || spec.identity.upgrade != 0
        || !crate::engine::play::body_enchantment_is_exact(spec)
        || crate::content_tables::card_row(spec.identity.id, spec.identity.upgrade)
            != Some(spec.row)
    {
        return false;
    }
    let [block, select] = catalog.steps(spec) else {
        return false;
    };
    block.kind == StepKind::Block
        && catalog.args(block.args) == [CompiledArg::I(5)]
        && select.kind == StepKind::Select
        && matches!(
            catalog.args(select.args),
            [
                CompiledArg::Pile(PileId::Hand),
                CompiledArg::I(1),
                CompiledArg::I(1),
                CompiledArg::Filter(crate::ids::FilterMode::Upgradable),
                CompiledArg::Select(crate::ids::SelectOp::Upgrade),
            ]
        )
}

/// Validate Armaments L0's complete selectable next-rung closure before any
/// spend, source move, Block, event, or pending publication.
pub(crate) fn preflight_upgrade_one_hand(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    source_will_be_in_play: bool,
) -> Result<(), EngineRefusal> {
    if !upgrade_one_hand_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs(
            "Armaments L0 upgrade selection preflight",
        ));
    }
    let sources = PileId::ALL
        .into_iter()
        .flat_map(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .map(move |card| (pile, *card))
        })
        .filter(|(_, card)| card.uid == source_uid)
        .collect::<Vec<_>>();
    if sources.len() != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: sources.len(),
        });
    }
    let (source_pile, source) = sources[0];
    if source_pile == PileId::Exhaust
        || source.flags & CARD_FLAG_LEGACY != 0
        || catalog.spec(source.atom) != Some(spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Armaments L0 upgrade selection source",
        ));
    }
    let frozen_uids = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .filter(|card| !source_will_be_in_play || card.uid != source_uid)
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    let mut probe = state.clone();
    upgrade_live_cards_once(&mut probe, catalog, &frozen_uids)
}

/// Validate Armaments+'s late frozen-Hand writer before any play prefix.
///
/// Manual play has not moved the source to Play at this seam, while collected
/// Sly AutoPlay intentionally retains its non-null Hand/Draw/Discard pile.
/// Authenticate that unique starting card and run the complete upgrade
/// transaction on a probe with the exact body-time Hand snapshot, so a bad
/// source or missing next atom cannot publish energy, Block, or listeners.
pub(crate) fn preflight_upgrade_all_hand(
    state: &HotState,
    catalog: &Catalog,
    source_uid: u32,
    spec: &CardSpec,
    source_will_be_in_play: bool,
) -> Result<(), EngineRefusal> {
    if !upgrade_all_hand_program_is_exact(catalog, spec) {
        return Err(EngineRefusal::MalformedArgs("upgrade_all_hand preflight"));
    }
    let sources = PileId::ALL
        .into_iter()
        .flat_map(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .map(move |card| (pile, *card))
        })
        .filter(|(_, card)| card.uid == source_uid)
        .collect::<Vec<_>>();
    if sources.len() != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: sources.len(),
        });
    }
    let (source_pile, source) = sources[0];
    if source_pile == PileId::Exhaust
        || source.flags & CARD_FLAG_LEGACY != 0
        || catalog.spec(source.atom) != Some(spec)
    {
        return Err(EngineRefusal::MalformedArgs(
            "upgrade_all_hand starting source",
        ));
    }
    let frozen_uids = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .filter(|card| !source_will_be_in_play || card.uid != source_uid)
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    let mut probe = state.clone();
    upgrade_live_cards_once(&mut probe, catalog, &frozen_uids)
}

pub(crate) fn upgrade_all_hand(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || !upgrade_all_hand_program_is_exact(ctx.catalog, ctx.spec)
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.x_value != 0
    {
        return Err(EngineRefusal::MalformedArgs("upgrade_all_hand"));
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
    if source_pile == PileId::Exhaust {
        return Err(EngineRefusal::MalformedArgs(
            "upgrade_all_hand active source",
        ));
    }
    let source = &ctx.state.piles.get(source_pile).as_slice()[source_index];
    if source.flags & CARD_FLAG_LEGACY != 0 || ctx.catalog.spec(source.atom) != Some(ctx.spec) {
        return Err(EngineRefusal::MalformedArgs(
            "upgrade_all_hand active source",
        ));
    }
    let frozen_uids = ctx
        .state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    upgrade_live_cards_once(ctx.state, ctx.catalog, &frozen_uids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CardSpec, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::engine::play::autoplay_collected_cards;
    use crate::engine::{Action, Event, SelectionRef, apply_action_into};
    use crate::hot::{CARD_FLAG_PICK, CardInstanceState, HotMonster, HotPile, HotState};
    use crate::ids::MonsterKind;
    use crate::powers::SlotWire;

    const OWNED: &[StepKind] = &[
        StepKind::AngerExact,
        StepKind::AttackPerStrike,
        StepKind::ExhaustRandom,
        StepKind::Havoc,
        StepKind::MoltenVuln,
        StepKind::ThunderclapExact,
        StepKind::UpgradeAllHand,
    ];

    fn identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn anger_fixture(upgrade: u8, hp: i32) -> (HotState, crate::catalog::Catalog, HotCard) {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity(CardId::Anger, upgrade)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        let source = HotCard {
            uid: 7,
            atom,
            flags: CARD_FLAG_PICK,
        };
        state.card_states.set(
            source.uid,
            CardInstanceState {
                damage_growth: 3,
                ..CardInstanceState::default()
            },
        );
        state.next_card_uid = 100;
        (state, catalog, source)
    }

    fn armaments_fixture() -> (HotState, crate::catalog::Catalog, [HotCard; 3]) {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::Armaments, 1)).unwrap();
        let apotheosis_atom = builder.intern(identity(CardId::Apotheosis, 0)).unwrap();
        let strike_atom = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        let catalog = builder.build();
        let cards = [
            HotCard {
                uid: 7,
                atom: source_atom,
                flags: CARD_FLAG_PICK,
            },
            HotCard {
                uid: 8,
                atom: apotheosis_atom,
                flags: CARD_FLAG_PICK,
            },
            HotCard {
                uid: 9,
                atom: strike_atom,
                flags: CARD_FLAG_PICK,
            },
        ];
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        (state, catalog, cards)
    }

    fn run_armaments_body(
        state: &mut HotState,
        catalog: &Catalog,
        spec: &CardSpec,
        source_uid: u32,
        operands: (Option<usize>, Option<u32>, i64, &[CompiledArg]),
    ) -> (Result<(), EngineRefusal>, Vec<Event>) {
        let (target, selection, x_value, args) = operands;
        let mut events = Vec::new();
        let mut ctx = StepCtx {
            state,
            catalog,
            spec,
            source_uid,
            target,
            selection,
            x_value,
            args,
            events: &mut events,
        };
        (upgrade_all_hand(&mut ctx), events)
    }

    #[test]
    fn manifest_claims_only_exactly_ported_kinds() {
        assert_eq!(
            IMPLEMENTED,
            &[
                StepKind::AngerExact,
                StepKind::AttackPerStrike,
                StepKind::ExhaustRandom,
                StepKind::Havoc,
                StepKind::MoltenVuln,
                StepKind::ThunderclapExact,
                StepKind::UpgradeAllHand,
            ]
        );
        assert!(IMPLEMENTED.contains(&StepKind::Havoc));
        assert!(IMPLEMENTED.contains(&StepKind::UpgradeAllHand));
    }

    #[test]
    fn all_fourteen_generated_carriers_and_operands_are_pinned() {
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
                (CardId::Anger, 0, StepKind::AngerExact, &[Arg::I(6)][..]),
                (CardId::Anger, 1, StepKind::AngerExact, &[Arg::I(8)][..]),
                (CardId::Armaments, 1, StepKind::UpgradeAllHand, &[][..],),
                (CardId::Cinder, 0, StepKind::ExhaustRandom, &[][..]),
                (CardId::Cinder, 1, StepKind::ExhaustRandom, &[][..]),
                (CardId::Havoc, 0, StepKind::Havoc, &[][..]),
                (CardId::Havoc, 1, StepKind::Havoc, &[][..]),
                (CardId::MoltenFist, 0, StepKind::MoltenVuln, &[][..]),
                (CardId::MoltenFist, 1, StepKind::MoltenVuln, &[][..]),
                (
                    CardId::PerfectedStrike,
                    0,
                    StepKind::AttackPerStrike,
                    &[Arg::I(6), Arg::I(2)][..],
                ),
                (
                    CardId::PerfectedStrike,
                    1,
                    StepKind::AttackPerStrike,
                    &[Arg::I(6), Arg::I(3)][..],
                ),
                (
                    CardId::Thunderclap,
                    0,
                    StepKind::ThunderclapExact,
                    &[Arg::I(4), Arg::I(1)][..],
                ),
                (
                    CardId::Thunderclap,
                    1,
                    StepKind::ThunderclapExact,
                    &[Arg::I(7), Arg::I(1)][..],
                ),
                (CardId::TrueGrit, 0, StepKind::ExhaustRandom, &[][..]),
            ]
        );
    }

    #[test]
    fn anger_reresolves_and_clones_the_exact_live_payload_from_every_pile() {
        for pile in PileId::ALL {
            let (mut state, catalog, source) = anger_fixture(0, 30);
            state.piles.set(pile, HotPile::from_cards(vec![source]));
            let spec = *catalog.spec(source.atom).unwrap();
            let args = [CompiledArg::I(6)];
            let mut events = Vec::new();
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

            anger_exact(&mut ctx).unwrap();

            assert_eq!(state.monsters[0].hp, 24, "{pile:?}");
            assert_eq!(state.history.owner_generated_cards_combat, 1, "{pile:?}");
            assert_eq!(state.next_generated_hook_uid, 1, "{pile:?}");
            assert_eq!(state.next_card_uid, 101, "{pile:?}");
            assert!(state.exact_piles, "{pile:?}");
            assert_eq!(
                state
                    .piles
                    .get(pile)
                    .as_slice()
                    .iter()
                    .filter(|card| **card == source)
                    .count(),
                1,
                "{pile:?}: the exact source remains live"
            );
            let clone = state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .find(|card| card.uid == 100)
                .copied()
                .unwrap();
            assert_eq!(clone.atom, source.atom, "{pile:?}");
            assert_eq!(clone.flags, source.flags, "{pile:?}");
            assert_eq!(
                state.card_states.get(clone.uid),
                state.card_states.get(source.uid),
                "{pile:?}"
            );
            assert!(matches!(
                events.last(),
                Some(Event::CardResolved {
                    uid: 100,
                    pile: PileId::Discard
                })
            ));
        }
    }

    #[test]
    fn lethal_anger_records_generation_without_allocating_or_inserting() {
        let (mut state, catalog, source) = anger_fixture(1, 8);
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![source]));
        state.history.owner_generated_cards_combat = 4;
        state.next_generated_hook_uid = 7;
        let spec = *catalog.spec(source.atom).unwrap();
        let args = [CompiledArg::I(8)];
        let mut events = Vec::new();
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

        anger_exact(&mut ctx).unwrap();

        assert!(state.history.over);
        assert_eq!(state.history.owner_generated_cards_combat, 5);
        assert_eq!(state.next_generated_hook_uid, 8);
        assert_eq!(state.next_card_uid, 100);
        assert!(state.piles.get(PileId::Discard).as_slice().is_empty());
        assert_eq!(state.piles.get(PileId::Play).as_slice(), &[source]);
        assert!(state.exact_piles);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::CardResolved { .. }))
        );
    }

    #[test]
    fn public_lethal_anger_records_exactly_one_generation_transaction() {
        let (mut state, catalog, source) = anger_fixture(1, 8);
        state.energy = 3;
        state
            .piles
            .set(PileId::Hand, HotPile::from_cards(vec![source]));
        state.history.owner_generated_cards_combat = 4;
        state.next_generated_hook_uid = 7;
        let mut events = Vec::new();

        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();

        assert!(next.history.over);
        assert_eq!(next.history.owner_generated_cards_combat, 5);
        assert_eq!(next.next_generated_hook_uid, 8);
        assert_eq!(next.next_card_uid, 100);
        assert!(next.piles.get(PileId::Discard).as_slice().is_empty());
        assert_eq!(next.piles.get(PileId::Play).as_slice(), &[source]);
        assert!(!PileId::ALL.into_iter().any(|pile| {
            next.piles
                .get(pile)
                .as_slice()
                .iter()
                .any(|card| card.uid == 100)
        }));
        assert_eq!(
            events,
            [
                Event::CardPlayed {
                    uid: source.uid,
                    atom: source.atom,
                    energy: 0,
                },
                Event::MonsterDamaged {
                    uid: 0,
                    blocked: 0,
                    unblocked: 8,
                    hp: 0,
                },
                Event::MonsterDied { uid: 0 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    /// #2974: the post-attack re-resolve accepts an L0 -> L1 drift only as
    /// the consumed Dampen row for this UID. A drift without that row, with a
    /// different restore atom, or with Dampen still live keeps refusing.
    #[test]
    fn anger_reresolve_accepts_only_a_consumed_dampen_restore() {
        let mut builder = CatalogBuilder::new();
        let l0 = builder.intern(identity(CardId::Anger, 0)).unwrap();
        let l1 = builder.intern(identity(CardId::Anger, 1)).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(l0).unwrap();
        let args = [CompiledArg::I(6)];
        let live = HotCard {
            uid: 7,
            atom: l1,
            flags: CARD_FLAG_PICK,
        };
        let mut state = HotState::at_defaults();
        state.next_card_uid = 8;
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![live]));

        let run = |state: &mut HotState, restored: Option<crate::catalog::CardAtom>| {
            let mut events = Vec::new();
            let ctx = StepCtx {
                state,
                catalog: &catalog,
                spec: &spec,
                source_uid: live.uid,
                target: Some(0),
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            };
            exact_active_anger(&ctx, restored)
        };

        assert_eq!(run(&mut state, Some(l1)), Ok(live));
        for restored in [None, Some(l0)] {
            assert_eq!(
                run(&mut state, restored),
                Err(EngineRefusal::MalformedArgs("anger_exact"))
            );
        }
        state.card_states.set_dampen(Some(crate::hot::DampenState {
            caster_uid: 2,
            cards: Vec::new(),
        }));
        assert_eq!(
            run(&mut state, Some(l1)),
            Err(EngineRefusal::MalformedArgs("anger_exact"))
        );
    }

    #[test]
    fn anger_requires_its_exact_fused_row_and_unique_live_source() {
        let (mut state, catalog, source) = anger_fixture(0, 30);
        state
            .piles
            .set(PileId::Play, HotPile::from_cards(vec![source]));
        state
            .piles
            .set(PileId::Discard, HotPile::from_cards(vec![source]));
        let before = state.clone();
        let spec = *catalog.spec(source.atom).unwrap();
        let args = [CompiledArg::I(6)];
        let mut events = Vec::new();
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
        assert_eq!(
            anger_exact(&mut ctx),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state
            .piles
            .set(PileId::Discard, HotPile::from_cards(Vec::new()));
        let bad_args = [CompiledArg::I(8)];
        let mut ctx = StepCtx {
            state: &mut state,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: Some(0),
            selection: None,
            x_value: 0,
            args: &bad_args,
            events: &mut events,
        };
        assert_eq!(
            anger_exact(&mut ctx),
            Err(EngineRefusal::MalformedArgs("anger_exact"))
        );
    }

    #[test]
    fn armaments_plus_public_play_upgrades_the_frozen_hand_in_place() {
        let (mut state, catalog, [source, apotheosis, strike]) = armaments_fixture();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis, strike]);
        let mut events = Vec::new();

        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();

        assert_eq!(next.block, 5);
        assert_eq!(next.energy, 2);
        assert_eq!(
            next.piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [apotheosis.uid, strike.uid]
        );
        for card in next.piles.get(PileId::Hand).as_slice() {
            assert_eq!(catalog.spec(card.atom).unwrap().identity.upgrade, 1);
        }
        assert_eq!(
            next.piles.get(PileId::Discard).as_slice(),
            &[source],
            "the physical source routes only after the complete body"
        );
    }

    #[test]
    fn armaments_plus_autoplay_uses_the_same_hand_snapshot_without_spending_energy() {
        let (mut state, catalog, [source, apotheosis, strike]) = armaments_fixture();
        state.piles.get_mut(PileId::Discard).make_mut().push(source);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([apotheosis, strike]);
        let mut events = Vec::new();

        autoplay_collected_cards(&mut state, &catalog, &[source], &mut events).unwrap();

        assert_eq!(state.energy, 3);
        assert_eq!(state.block, 5);
        assert!(state.piles.get(PileId::Hand).as_slice().iter().all(|card| {
            catalog
                .spec(card.atom)
                .is_some_and(|spec| spec.identity.upgrade == 1)
        }));
        assert_eq!(state.piles.get(PileId::Discard).as_slice(), &[source]);
    }

    #[test]
    fn armaments_plus_terminal_block_listener_suppresses_the_upgrade_suffix() {
        let (mut state, catalog, [source, apotheosis, _]) = armaments_fixture();
        state.monsters_mut()[0].hp = 5;
        state.monsters_mut()[0].max_hp = 5;
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis]);
        let original_atom = apotheosis.atom;
        let mut events = Vec::new();

        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::new(None),
            },
            &mut events,
        )
        .unwrap();

        assert!(next.history.over);
        assert_eq!(
            next.piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .find(|card| card.uid == apotheosis.uid)
                .unwrap()
                .atom,
            original_atom
        );
    }

    #[test]
    fn armaments_plus_block_overflow_and_malformed_body_are_atomic() {
        let (mut state, catalog, [source, apotheosis, _]) = armaments_fixture();
        state.block = i32::MAX - 4;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis]);
        let before = state.clone();
        let mut events = Vec::new();
        assert!(
            apply_action_into(
                &state,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut events,
            )
            .is_err()
        );
        assert_eq!(state, before);

        let spec = *catalog.spec(source.atom).unwrap();
        let bad_args = [CompiledArg::I(1)];
        let mut direct = before.clone();
        let direct_before = direct.clone();
        let mut direct_events = Vec::new();
        let mut ctx = StepCtx {
            state: &mut direct,
            catalog: &catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &bad_args,
            events: &mut direct_events,
        };
        assert_eq!(
            upgrade_all_hand(&mut ctx),
            Err(EngineRefusal::MalformedArgs("upgrade_all_hand"))
        );
        assert_eq!(direct, direct_before);
        assert!(direct_events.is_empty());
    }

    #[test]
    fn armaments_plus_preflights_source_and_full_hand_closure_before_any_play_prefix() {
        let (mut duplicate, catalog, [source, apotheosis, _]) = armaments_fixture();
        duplicate
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis]);
        duplicate
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(source);
        let duplicate_before = duplicate.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &duplicate,
                &catalog,
                &Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut events,
            ),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: source.uid,
                matches: 2,
            })
        );
        assert_eq!(duplicate, duplicate_before);
        assert!(events.is_empty(), "Block must not precede source refusal");

        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(identity(CardId::Armaments, 1)).unwrap();
        let apotheosis_atom = builder.intern(identity(CardId::Apotheosis, 0)).unwrap();
        let incomplete = builder.build();
        let source = HotCard {
            uid: 17,
            atom: source_atom,
            flags: CARD_FLAG_PICK,
        };
        let apotheosis = HotCard {
            uid: 18,
            atom: apotheosis_atom,
            flags: CARD_FLAG_PICK,
        };
        let missing = CardIdentity {
            id: CardId::Apotheosis,
            upgrade: 1,
            enchantment: None,
        };

        let mut manual = HotState::at_defaults();
        manual.hp = 50;
        manual.energy = 3;
        manual
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        manual
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([source, apotheosis]);
        let manual_before = manual.clone();
        let mut manual_events = Vec::new();
        assert_eq!(
            apply_action_into(
                &manual,
                &incomplete,
                &Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut manual_events,
            ),
            Err(EngineRefusal::UnknownMintIdentity(missing))
        );
        assert_eq!(manual, manual_before);
        assert!(
            manual_events.is_empty(),
            "energy, source movement, CardPlayed, and Block stay unpublished"
        );

        let mut direct = manual_before.clone();
        direct.piles.get_mut(PileId::Hand).make_mut().remove(0);
        direct.piles.get_mut(PileId::Play).make_mut().push(source);
        let direct_before = direct.clone();
        let direct_spec = *incomplete.spec(source.atom).unwrap();
        assert_eq!(
            preflight_upgrade_all_hand(&direct, &incomplete, source.uid, &direct_spec, true),
            Err(EngineRefusal::UnknownMintIdentity(missing))
        );
        assert_eq!(direct, direct_before);

        let mut nested = manual_before;
        nested.piles.get_mut(PileId::Hand).make_mut().remove(0);
        nested
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(source);
        let nested_before = nested.clone();
        let mut nested_events = Vec::new();
        assert_eq!(
            autoplay_collected_cards(&mut nested, &incomplete, &[source], &mut nested_events,),
            Err(EngineRefusal::UnknownMintIdentity(missing))
        );
        assert_eq!(nested, nested_before);
        assert!(nested_events.is_empty());
    }

    #[test]
    fn armaments_plus_body_authenticates_action_program_and_unique_play_source_atomically() {
        let (state, catalog, [source, apotheosis, _]) = armaments_fixture();
        let spec = *catalog.spec(source.atom).unwrap();

        for (target, selection, x_value, args) in [
            (Some(0), None, 0, &[][..]),
            (None, Some(apotheosis.uid), 0, &[][..]),
            (None, None, 1, &[][..]),
            (None, None, 0, &[CompiledArg::I(1)][..]),
        ] {
            let mut candidate = state.clone();
            candidate
                .piles
                .get_mut(PileId::Play)
                .make_mut()
                .push(source);
            candidate
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(apotheosis);
            let before = candidate.clone();
            let (result, events) = run_armaments_body(
                &mut candidate,
                &catalog,
                &spec,
                source.uid,
                (target, selection, x_value, args),
            );
            assert_eq!(
                result,
                Err(EngineRefusal::MalformedArgs("upgrade_all_hand"))
            );
            assert_eq!(candidate, before);
            assert!(events.is_empty());
        }

        let mut forged_spec = spec;
        forged_spec.row = crate::content_tables::card_row(CardId::Armaments, 0).unwrap();
        let mut forged = state.clone();
        forged.piles.get_mut(PileId::Play).make_mut().push(source);
        let before = forged.clone();
        let (result, events) = run_armaments_body(
            &mut forged,
            &catalog,
            &forged_spec,
            source.uid,
            (None, None, 0, &[]),
        );
        assert_eq!(
            result,
            Err(EngineRefusal::MalformedArgs("upgrade_all_hand"))
        );
        assert_eq!(forged, before);
        assert!(events.is_empty());

        for location in [None, Some(PileId::Exhaust), Some(PileId::Play)] {
            let mut candidate = state.clone();
            if let Some(pile) = location {
                candidate.piles.get_mut(pile).make_mut().push(source);
            }
            if location == Some(PileId::Play) {
                candidate
                    .piles
                    .get_mut(PileId::Draw)
                    .make_mut()
                    .push(source);
            }
            let before = candidate.clone();
            let (result, events) = run_armaments_body(
                &mut candidate,
                &catalog,
                &spec,
                source.uid,
                (None, None, 0, &[]),
            );
            assert!(result.is_err());
            assert_eq!(candidate, before);
            assert!(events.is_empty());
        }

        for location in [PileId::Hand, PileId::Draw, PileId::Discard, PileId::Play] {
            let mut candidate = state.clone();
            candidate.piles.get_mut(location).make_mut().push(source);
            candidate
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(apotheosis);
            let (result, events) = run_armaments_body(
                &mut candidate,
                &catalog,
                &spec,
                source.uid,
                (None, None, 0, &[]),
            );
            assert_eq!(result, Ok(()));
            assert!(events.is_empty());
            let (live_pile, live_index) =
                crate::engine::play::unique_live_card_location(&candidate, source.uid)
                    .unwrap()
                    .unwrap();
            assert_eq!(live_pile, location);
            assert_eq!(
                candidate.piles.get(live_pile).as_slice()[live_index].uid,
                source.uid
            );
        }

        let mut wrong_atom = state.clone();
        let mut wrong_source = source;
        wrong_source.atom = apotheosis.atom;
        wrong_atom
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(wrong_source);
        let before = wrong_atom.clone();
        let (result, events) = run_armaments_body(
            &mut wrong_atom,
            &catalog,
            &spec,
            source.uid,
            (None, None, 0, &[]),
        );
        assert_eq!(
            result,
            Err(EngineRefusal::MalformedArgs(
                "upgrade_all_hand active source"
            ))
        );
        assert_eq!(wrong_atom, before);
        assert!(events.is_empty());
    }

    #[test]
    fn family_admission_census_is_fourteen_after_armaments_upgrade_closure() {
        let family_rows = CARD_ROWS
            .iter()
            .filter(|row| row.steps.iter().any(|step| OWNED.contains(&step.kind)))
            .collect::<Vec<_>>();
        assert_eq!(family_rows.len(), 14);
        let admitted = family_rows
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .all(|step| crate::steps::is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(
            admitted,
            vec![
                (CardId::Anger, 0),
                (CardId::Anger, 1),
                (CardId::Armaments, 1),
                (CardId::Cinder, 0),
                (CardId::Cinder, 1),
                (CardId::Havoc, 0),
                (CardId::Havoc, 1),
                (CardId::MoltenFist, 0),
                (CardId::MoltenFist, 1),
                (CardId::PerfectedStrike, 0),
                (CardId::PerfectedStrike, 1),
                (CardId::Thunderclap, 0),
                (CardId::Thunderclap, 1),
                (CardId::TrueGrit, 0),
            ]
        );
        let anger_delta = family_rows
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::AngerExact)
                    && row
                        .steps
                        .iter()
                        .all(|step| crate::steps::is_implemented(step.kind))
            })
            .map(|row| (row.id, row.upgrade))
            .collect::<Vec<_>>();
        assert_eq!(anger_delta, vec![(CardId::Anger, 0), (CardId::Anger, 1)]);
        assert!(crate::steps::is_implemented(StepKind::AngerExact));
        assert!(crate::steps::is_implemented(StepKind::Havoc));
        assert!(crate::steps::is_implemented(StepKind::UpgradeAllHand));
        let manifest = crate::engine::capability_manifest();
        assert!(manifest.steps.contains(&StepKind::AngerExact));
        assert!(manifest.steps.contains(&StepKind::Havoc));
        assert!(manifest.steps.contains(&StepKind::UpgradeAllHand));
    }

    /// Cinder/True Grit in Hand with two bystanders, one Toadpole at `hp`.
    fn exhaust_random_fixture(
        source: CardIdentity,
        hp: i32,
    ) -> (HotState, Catalog, HotCard, [HotCard; 2]) {
        let mut builder = CatalogBuilder::new();
        let source_atom = builder.intern(source).unwrap();
        let strike = builder.intern(identity(CardId::StrikeIronclad, 0)).unwrap();
        let defend = builder.intern(identity(CardId::DefendIronclad, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 90,
            atom: source_atom,
            flags: 0,
        };
        let bystanders = [
            HotCard {
                uid: 7,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 8,
                atom: defend,
                flags: 0,
            },
        ];
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.next_card_uid = 100;
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: [13, 14, 15, 16],
                counter: 0,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        hand.push(source);
        hand.extend(bystanders);
        (state, catalog, source, bystanders)
    }

    fn pile(state: &HotState, pile: PileId) -> Vec<HotCard> {
        state.piles.get(pile).as_slice().to_vec()
    }

    #[test]
    fn killing_cinder_spends_the_selection_draw_but_leaves_every_pile_unchanged() {
        // #3041: Cinder/<OnPlay>d__5 (0x392350) IL_011b–0143 always consumes
        // the NextItem draw; CardCmd.Exhaust (0x3e06c8) IL_0020–0034 then
        // returns on IsOverOrEnding before Add, so the pick stays in Hand.
        let (mut state, catalog, source, bystanders) =
            exhaust_random_fixture(identity(CardId::Cinder, 0), 1);

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
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(pile(&state, PileId::Hand), bystanders.to_vec());
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert!(state.piles.get(PileId::Draw).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    #[test]
    fn nonlethal_cinder_still_exhausts_the_selected_hand_card() {
        let (mut state, catalog, source, bystanders) =
            exhaust_random_fixture(identity(CardId::Cinder, 0), 1_000);

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            Some(0),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(!state.history.over);
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        let exhausted = pile(&state, PileId::Exhaust);
        assert_eq!(exhausted.len(), 1);
        assert!(bystanders.contains(&exhausted[0]));
        let mut survivors = pile(&state, PileId::Hand);
        survivors.push(exhausted[0]);
        survivors.sort_by_key(|card| card.uid);
        assert_eq!(survivors, bystanders.to_vec());
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
    }

    #[test]
    fn glam_cinder_whose_second_play_kills_keeps_the_second_pick_in_hand() {
        // First body (18 damage) leaves the Toadpole alive and exhausts one
        // bystander; the Glam replay kills, spends a second Selection draw,
        // and its gated Exhaust leaves the other bystander in Hand.
        let glam_cinder = CardIdentity {
            id: CardId::Cinder,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let (mut state, catalog, source, bystanders) = exhaust_random_fixture(glam_cinder, 30);

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
        assert_eq!(state.rng.get(RngStream::Sel).counter, 2);
        let exhausted = pile(&state, PileId::Exhaust);
        let hand = pile(&state, PileId::Hand);
        assert_eq!(exhausted.len(), 1, "only the first body's pick exhausts");
        assert_eq!(hand.len(), 1, "the killing body's pick stays in Hand");
        let mut all = [exhausted[0], hand[0]];
        all.sort_by_key(|card| card.uid);
        assert_eq!(all, bystanders);
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
    }

    #[test]
    fn true_grit_juggernaut_killing_blow_leaves_the_pick_in_hand() {
        // TrueGrit/<OnPlay>d__7 (0x3c4950) IL_01af–01d9 picks after the
        // Block command; a Juggernaut kill ends combat first, and the
        // gated CardCmd.Exhaust (IL_01ee) leaves the pick in Hand.
        let (mut state, catalog, source, bystanders) =
            exhaust_random_fixture(identity(CardId::TrueGrit, 0), 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .register_after_block_gained(PowerId::Juggernaut)
        );

        crate::engine::play::play_card(
            &mut state,
            &catalog,
            source.uid,
            None,
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(pile(&state, PileId::Hand), bystanders.to_vec());
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }
}
