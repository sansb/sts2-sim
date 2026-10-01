//! Card-step bodies for the `content/cards/hand_cap.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # Wave status (#1363 Batch 6): exact body and ten-row closure admitted
//!
//! The single generated kind is a closed five-mode family, not five
//! independently claimable capabilities. This foundation transcribes all
//! five bodies, derives rarity from the generated card table, closes Crash
//! Landing's Debris mint, and returns a family-owned cardinality descriptor
//! for the two blocking Discard selectors. Unit E packs that descriptor into
//! the shared continuation wire, re-derives the live Discard candidates, and
//! admits all ten exact rows together.

use super::StepCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::CardRarity;
use crate::engine::damage::{player_attack_all_from_card, player_attack_from_card};
use crate::engine::draw::MAX_CARDS_IN_HAND;
use crate::engine::{EngineRefusal, Event};
use crate::hot::{HotPile, PileId, RngStream, RngStreamState};
use crate::ids::{CardId, StepKind, StepWord};
use crate::rng::Xoshiro256StarStar;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::HandCapBody];

/// A body-owned Discard selector that the shared continuation layer must
/// suspend and later resolve.
///
/// The source pile and move operation are invariant across both selector
/// modes. Keeping only the native cardinality here lets the family body be
/// source-exact without depending on the concurrently packed pending wire.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandCapSelection {
    pub(crate) min: usize,
    pub(crate) max: usize,
}

/// Exact current-build carrier closure for the fused five-mode body.
pub(crate) fn program_is_supported(row: &crate::content_tables::CardRow) -> bool {
    let [step] = row.steps else {
        return false;
    };
    if step.kind != StepKind::HandCapBody
        || crate::content_tables::card_row(row.id, row.upgrade) != Some(row)
    {
        return false;
    }
    matches!(
        (row.id, row.upgrade, step.args),
        (
            CardId::Anointed,
            0 | 1,
            [crate::content_tables::Arg::S("anointed_rare")]
        ) | (
            CardId::CrashLanding,
            0,
            [
                crate::content_tables::Arg::S("aoe_debris"),
                crate::content_tables::Arg::I(21)
            ]
        ) | (
            CardId::CrashLanding,
            1,
            [
                crate::content_tables::Arg::S("aoe_debris"),
                crate::content_tables::Arg::I(26)
            ]
        ) | (
            CardId::Dredge,
            0 | 1,
            [
                crate::content_tables::Arg::S("discard_fixed"),
                crate::content_tables::Arg::I(3)
            ]
        ) | (
            CardId::NeowsFury,
            0,
            [
                crate::content_tables::Arg::S("attack_discard_optional"),
                crate::content_tables::Arg::I(10),
                crate::content_tables::Arg::I(2)
            ]
        ) | (
            CardId::NeowsFury,
            1,
            [
                crate::content_tables::Arg::S("attack_discard_optional"),
                crate::content_tables::Arg::I(14),
                crate::content_tables::Arg::I(3)
            ]
        ) | (
            CardId::Scrawl,
            0 | 1,
            [crate::content_tables::Arg::S("draw_space")]
        )
    )
}

/// Re-derive the two blocking selector cardinalities from the authoritative
/// row and then-live Hand space. `None` means this row is synchronous or is
/// not one of the exact current-build HandCap carriers.
pub(crate) fn pending_selection_for_row(
    row: &crate::content_tables::CardRow,
    space: usize,
) -> Option<HandCapSelection> {
    if !program_is_supported(row) {
        return None;
    }
    match (row.id, row.upgrade) {
        (CardId::Dredge, 0 | 1) => {
            let amount = 3.min(space);
            (amount > 0).then_some(HandCapSelection {
                min: amount,
                max: amount,
            })
        }
        (CardId::NeowsFury, 0) => {
            let amount = 2.min(space);
            (amount > 0).then_some(HandCapSelection {
                min: 0,
                max: amount,
            })
        }
        (CardId::NeowsFury, 1) => {
            let amount = 3.min(space);
            (amount > 0).then_some(HandCapSelection {
                min: 0,
                max: amount,
            })
        }
        _ => None,
    }
}

fn validate_owner(ctx: &StepCtx<'_>) -> Result<StepWord, EngineRefusal> {
    let Some(step) = ctx.catalog.steps(ctx.spec).first() else {
        return Err(EngineRefusal::MalformedArgs("hand_cap_body program"));
    };
    if ctx.catalog.steps(ctx.spec).len() != 1
        || step.kind != StepKind::HandCapBody
        || ctx.catalog.args(step.args) != ctx.args
        || crate::content_tables::card_row(ctx.spec.identity.id, ctx.spec.identity.upgrade)
            != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("hand_cap_body program"));
    }
    let expected = match (ctx.spec.identity.id, ctx.spec.identity.upgrade) {
        (CardId::Anointed, 0 | 1) => &[CompiledArg::Word(StepWord::AnointedRare)][..],
        (CardId::CrashLanding, 0) => &[CompiledArg::Word(StepWord::AoeDebris), CompiledArg::I(21)],
        (CardId::CrashLanding, 1) => &[CompiledArg::Word(StepWord::AoeDebris), CompiledArg::I(26)],
        (CardId::Dredge, 0 | 1) => &[CompiledArg::Word(StepWord::DiscardFixed), CompiledArg::I(3)],
        (CardId::NeowsFury, 0) => &[
            CompiledArg::Word(StepWord::AttackDiscardOptional),
            CompiledArg::I(10),
            CompiledArg::I(2),
        ],
        (CardId::NeowsFury, 1) => &[
            CompiledArg::Word(StepWord::AttackDiscardOptional),
            CompiledArg::I(14),
            CompiledArg::I(3),
        ],
        (CardId::Scrawl, 0 | 1) => &[CompiledArg::Word(StepWord::DrawSpace)],
        _ => return Err(EngineRefusal::MalformedArgs("hand_cap_body owner")),
    };
    if ctx.args != expected {
        return Err(EngineRefusal::MalformedArgs("hand_cap_body row"));
    }
    match expected[0] {
        CompiledArg::Word(mode) => Ok(mode),
        _ => unreachable!("hand-cap rows begin with their compiled mode"),
    }
}

fn live_hand_space(ctx: &StepCtx<'_>) -> usize {
    MAX_CARDS_IN_HAND.saturating_sub(ctx.state.piles.get(PileId::Hand).len())
}

/// Begin one exact hand-cap body.
///
/// Synchronous modes finish here. A returned descriptor is the only part the
/// shared Unit-E continuation seam still needs to pack; Dredge's whole-pile
/// auto-answer is applied here before any suspension is considered.
///
/// Ending gates (#3515), each the shared IsEnding / IsOverOrEnding projection:
/// - Anointed (`Anointed/<OnPlay>d__3` `0x389eac`) rolls `TakeRandom` (IL_0088)
///   and then awaits `CardPileCmd.Add` to Hand (IL_0098); `<Add>d__10`
///   (`0x3e1ba4`) skips every combat-pile move at `IsEnding` (IL_0041-008a).
/// - Dredge (`0x39a30c`) and Neow's Fury (`0x3ae88c`, after its attack) await
///   `CardSelectCmd.FromCombatPile` (IL_0103 / IL_013d), whose
///   `<FromCombatPile>d__20` (`0x3e5e84`) returns no card at `IsEnding`
///   (IL_0036-003d), and then the same `CardPileCmd.Add`.
pub(crate) fn begin_hand_cap_body(
    ctx: &mut StepCtx<'_>,
) -> Result<Option<HandCapSelection>, EngineRefusal> {
    let mode = validate_owner(ctx)?;
    match mode {
        StepWord::AnointedRare => {
            let mut rare_indices = Vec::new();
            for (index, card) in ctx
                .state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .enumerate()
            {
                let spec = ctx
                    .catalog
                    .spec(card.atom)
                    .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
                if spec.row.rarity == CardRarity::Rare {
                    rare_indices.push(index);
                }
            }
            let live = ctx.state.rng.get(RngStream::Sel);
            let mut rng = Xoshiro256StarStar {
                words: live.words,
                counter: live.counter,
            };
            rng.shuffle(&mut rare_indices)
                .map_err(|_| EngineRefusal::CounterOverflow("Anointed shuffle"))?;
            ctx.state.rng.set(
                RngStream::Sel,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            let chosen_indices = &rare_indices[..live_hand_space(ctx).min(rare_indices.len())];
            if !crate::engine::damage::damage_combat_is_ending(ctx.state)
                && !chosen_indices.is_empty()
            {
                let draw = ctx.state.piles.get(PileId::Draw).as_slice();
                let chosen: Vec<_> = chosen_indices.iter().map(|index| draw[*index]).collect();
                let remaining: Vec<_> = draw
                    .iter()
                    .copied()
                    .enumerate()
                    .filter_map(|(index, card)| (!chosen_indices.contains(&index)).then_some(card))
                    .collect();
                ctx.state
                    .piles
                    .set(PileId::Draw, HotPile::from_cards(remaining));
                ctx.state
                    .piles
                    .get_mut(PileId::Hand)
                    .make_mut()
                    .extend(chosen);
            }
            Ok(None)
        }
        StepWord::AoeDebris => {
            let [_, CompiledArg::I(damage)] = ctx.args else {
                unreachable!("owner validation fixed the Crash Landing row")
            };
            let identity = CardIdentity {
                id: CardId::Debris,
                upgrade: 0,
                enchantment: None,
            };
            let leaf = ctx
                .catalog
                .atom(&identity)
                .and_then(|atom| ctx.catalog.spec(atom))
                .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
            if leaf.identity != identity || leaf.row.rarity != CardRarity::Status {
                return Err(EngineRefusal::MalformedArgs("Crash Landing Debris leaf"));
            }
            player_attack_all_from_card(
                ctx.state,
                (ctx.catalog, ctx.spec, ctx.source_uid),
                *damage,
                1,
                ctx.events,
            )?;
            let amount = live_hand_space(ctx);
            crate::engine::cards::inject_generated_record_before_ending_bottom(
                ctx.state,
                ctx.catalog,
                identity,
                amount,
                PileId::Hand,
                ctx.events,
            )?;
            Ok(None)
        }
        StepWord::DiscardFixed => {
            if crate::engine::damage::damage_combat_is_ending(ctx.state) {
                return Ok(None);
            }
            let Some(selection) = pending_selection_for_row(ctx.spec.row, live_hand_space(ctx))
            else {
                return Ok(None);
            };
            let candidate_count = ctx.state.piles.get(PileId::Discard).len();
            if candidate_count <= selection.min {
                let moved = ctx.state.piles.get(PileId::Discard).as_slice().to_vec();
                ctx.state
                    .piles
                    .set(PileId::Discard, HotPile::from_cards(Vec::new()));
                ctx.state
                    .piles
                    .get_mut(PileId::Hand)
                    .make_mut()
                    .extend(moved.iter().copied());
                ctx.events
                    .extend(moved.into_iter().map(|card| Event::CardResolved {
                        uid: card.uid,
                        pile: PileId::Hand,
                    }));
                Ok(None)
            } else {
                Ok(Some(selection))
            }
        }
        StepWord::AttackDiscardOptional => {
            let [_, CompiledArg::I(damage), CompiledArg::I(_)] = ctx.args else {
                unreachable!("owner validation fixed the Neow's Fury row")
            };
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
                return Ok(None);
            }
            let selection = pending_selection_for_row(ctx.spec.row, live_hand_space(ctx));
            if selection.is_none() || ctx.state.piles.get(PileId::Discard).is_empty() {
                Ok(None)
            } else {
                Ok(selection)
            }
        }
        StepWord::DrawSpace => {
            crate::engine::play::draw_cardplay_no_result(ctx, live_hand_space(ctx))?;
            Ok(None)
        }
        _ => Err(EngineRefusal::MalformedArgs("hand_cap_body mode")),
    }
}

/// `hand_cap_body` — body-exact and manifest-admitted.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) admits exactly both upgrade rows of
/// Anointed, Crash Landing, Dredge, Neow's Fury, and Scrawl, then dispatches
/// five modes. They respectively
/// shuffle and take Rare Draw cards, await powered AoE and mint Debris, move an
/// exact or optional variable-sized Discard selection, and run one ordinary
/// Draw command for the live post-removal space.
///
/// `add_fixed_generated_status` (frozen Python, deleted #2827) owns the record-before-ending Status
/// transaction. `_select_candidates` and `_apply_select_op` own
/// the selection's candidate snapshot and move-to-Hand consumer.
///
/// The shared wire carries [`HandCapSelection`]'s bounded cardinality, derives
/// its only legal source/operation from the authoritative row, and resumes via
/// the existing move-to-Hand consumer. Admission recognizes only the ten rows
/// pinned below and allows exact Debris+0 only with Crash Landing provenance.
pub(crate) fn hand_cap_body(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if begin_hand_cap_body(ctx)?.is_some() {
        Err(EngineRefusal::ContinuationNotModeled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardAtom, Catalog, CatalogBuilder};
    use crate::content_tables::{Arg, CARD_ROWS};
    use crate::hot::{HotCard, HotMonster, HotState};
    use crate::ids::MonsterKind;

    fn plain(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    fn card(uid: u32, atom: CardAtom) -> HotCard {
        HotCard {
            uid,
            atom,
            flags: 0,
        }
    }

    fn begin(
        state: &mut HotState,
        catalog: &Catalog,
        identity: CardIdentity,
        target: Option<usize>,
        events: &mut Vec<Event>,
    ) -> Result<Option<HandCapSelection>, EngineRefusal> {
        let atom = catalog.atom(&identity).unwrap();
        let spec = *catalog.spec(atom).unwrap();
        let step = catalog.steps(&spec)[0];
        let args = catalog.args(step.args).to_vec();
        begin_hand_cap_body(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: 900,
            target,
            selection: None,
            x_value: 0,
            args: &args,
            events,
        })
    }

    #[test]
    fn hand_cap_family_publishes_its_single_atomic_body() {
        assert_eq!(IMPLEMENTED, &[StepKind::HandCapBody]);
        assert!(crate::steps::is_implemented(StepKind::HandCapBody));
        assert!(
            crate::engine::capability_manifest()
                .steps
                .contains(&StepKind::HandCapBody)
        );
    }

    #[test]
    fn all_five_modes_and_ten_generated_carriers_have_exact_supported_rows() {
        let carriers: Vec<_> = CARD_ROWS
            .iter()
            .filter(|row| {
                row.steps
                    .iter()
                    .any(|step| step.kind == StepKind::HandCapBody)
            })
            .collect();

        assert_eq!(carriers.len(), 10, "five exact cards at both levels");
        let actual: Vec<_> = carriers
            .iter()
            .map(|row| {
                assert_eq!(row.steps.len(), 1, "{} has one atomic body", row.name);
                assert!(program_is_supported(row));
                (row.id, row.upgrade, row.steps[0].args)
            })
            .collect();
        assert_eq!(
            actual,
            vec![
                (CardId::Anointed, 0, &[Arg::S("anointed_rare")][..]),
                (CardId::Anointed, 1, &[Arg::S("anointed_rare")][..]),
                (
                    CardId::CrashLanding,
                    0,
                    &[Arg::S("aoe_debris"), Arg::I(21)][..],
                ),
                (
                    CardId::CrashLanding,
                    1,
                    &[Arg::S("aoe_debris"), Arg::I(26)][..],
                ),
                (CardId::Dredge, 0, &[Arg::S("discard_fixed"), Arg::I(3)][..],),
                (CardId::Dredge, 1, &[Arg::S("discard_fixed"), Arg::I(3)][..],),
                (
                    CardId::NeowsFury,
                    0,
                    &[Arg::S("attack_discard_optional"), Arg::I(10), Arg::I(2),][..],
                ),
                (
                    CardId::NeowsFury,
                    1,
                    &[Arg::S("attack_discard_optional"), Arg::I(14), Arg::I(3),][..],
                ),
                (CardId::Scrawl, 0, &[Arg::S("draw_space")][..]),
                (CardId::Scrawl, 1, &[Arg::S("draw_space")][..]),
            ]
        );
    }

    #[test]
    fn anointed_shuffles_every_rare_index_then_moves_only_live_space() {
        let owner = plain(CardId::Anointed, 0);
        let rare = plain(CardId::Scrawl, 0);
        let common = plain(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let owner_atom = builder.intern(owner).unwrap();
        let rare_atom = builder.intern(rare).unwrap();
        let common_atom = builder.intern(common).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: Xoshiro256StarStar::from_seed(17).words,
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(1, rare_atom),
            card(2, common_atom),
            card(3, rare_atom),
        ]);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((10..19).map(|uid| card(uid, owner_atom)));

        assert_eq!(
            begin(&mut state, &catalog, owner, None, &mut Vec::new()).unwrap(),
            None
        );
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .last()
                .unwrap()
                .atom,
            rare_atom
        );
        assert_eq!(state.piles.get(PileId::Draw).len(), 2);
        assert!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .any(|card| card.atom == common_atom)
        );
        assert!(!state.exact_piles);
    }

    #[test]
    fn terminal_anointed_still_consumes_the_complete_shuffle_without_moving() {
        let owner = plain(CardId::Anointed, 1);
        let rare = plain(CardId::Scrawl, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(owner).unwrap();
        let rare_atom = builder.intern(rare).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: Xoshiro256StarStar::from_seed(23).words,
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            card(1, rare_atom),
            card(2, rare_atom),
            card(3, rare_atom),
        ]);
        state.history.over = true;
        let before_draw = state.piles.get(PileId::Draw).clone();

        begin(&mut state, &catalog, owner, None, &mut Vec::new()).unwrap();

        assert_eq!(state.rng.get(RngStream::Sel).counter, 2);
        assert_eq!(state.piles.get(PileId::Draw), &before_draw);
        assert!(state.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn crash_landing_mints_debris_live_and_records_all_terminal_commands() {
        let owner = plain(CardId::CrashLanding, 0);
        let debris = plain(CardId::Debris, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(owner).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let debris_atom = catalog.atom(&debris).unwrap();

        let mut live = HotState::at_defaults();
        live.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let filler = catalog.atom(&owner).unwrap();
        live.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..9).map(|uid| card(uid, filler)));
        live.next_card_uid = 20;
        begin(&mut live, &catalog, owner, None, &mut Vec::new()).unwrap();
        assert_eq!(live.monsters[0].hp, 29);
        assert_eq!(live.history.owner_generated_cards_combat, 1);
        assert_eq!(live.piles.get(PileId::Hand).as_slice()[9].atom, debris_atom);

        let mut lethal = HotState::at_defaults();
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        lethal.next_card_uid = 70;
        begin(&mut lethal, &catalog, owner, None, &mut Vec::new()).unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.history.owner_generated_cards_combat, 10);
        assert_eq!(lethal.next_generated_hook_uid, 10);
        assert_eq!(lethal.next_card_uid, 70);
        assert!(lethal.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn dredge_auto_moves_the_whole_pile_or_returns_an_exact_count_selector() {
        let owner = plain(CardId::Dredge, 0);
        let filler = plain(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(owner).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        let catalog = builder.build();

        let mut auto = HotState::at_defaults();
        auto.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        auto.piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend([card(1, filler_atom), card(2, filler_atom)]);
        let mut events = Vec::new();
        assert_eq!(
            begin(&mut auto, &catalog, owner, None, &mut events).unwrap(),
            None
        );
        assert!(auto.piles.get(PileId::Discard).is_empty());
        assert_eq!(auto.piles.get(PileId::Hand).len(), 2);
        assert_eq!(events.len(), 2);

        let mut selecting = HotState::at_defaults();
        selecting.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        selecting
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((1..=4).map(|uid| card(uid, filler_atom)));
        let before = selecting.clone();
        assert_eq!(
            begin(&mut selecting, &catalog, owner, None, &mut Vec::new()).unwrap(),
            Some(HandCapSelection { min: 3, max: 3 })
        );
        assert_eq!(selecting, before);
    }

    #[test]
    fn neows_fury_attacks_before_opening_the_optional_discard_selector() {
        let owner = plain(CardId::NeowsFury, 1);
        let filler = plain(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        builder.intern(owner).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(card(1, filler_atom));

        assert_eq!(
            begin(&mut state, &catalog, owner, Some(0), &mut Vec::new()).unwrap(),
            Some(HandCapSelection { min: 0, max: 3 })
        );
        assert_eq!(state.monsters[0].hp, 16);

        let mut lethal = HotState::at_defaults();
        lethal
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        lethal
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(card(2, filler_atom));
        assert_eq!(
            begin(&mut lethal, &catalog, owner, Some(0), &mut Vec::new()).unwrap(),
            None
        );
        assert!(lethal.history.over);
        assert_eq!(lethal.piles.get(PileId::Discard).len(), 1);
    }

    #[test]
    fn scrawl_issues_one_command_for_exact_live_space() {
        let owner = plain(CardId::Scrawl, 0);
        let filler = plain(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let owner_atom = builder.intern(owner).unwrap();
        let filler_atom = builder.intern(filler).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((0..8).map(|uid| card(uid, owner_atom)));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((20..23).map(|uid| card(uid, filler_atom)));

        begin(&mut state, &catalog, owner, None, &mut Vec::new()).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), 10);
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
        assert_eq!(state.history.non_hand_draws_this_turn, 2);
    }

    #[test]
    fn malformed_hand_cap_arguments_refuse_before_mutation() {
        let owner = plain(CardId::Dredge, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(owner).unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        let before = state.clone();
        let mut events = Vec::new();
        let args = [CompiledArg::Word(StepWord::DiscardFixed), CompiledArg::I(4)];

        assert_eq!(
            begin_hand_cap_body(&mut StepCtx {
                state: &mut state,
                catalog: &catalog,
                spec: &spec,
                source_uid: 900,
                target: None,
                selection: None,
                x_value: 0,
                args: &args,
                events: &mut events,
            }),
            Err(EngineRefusal::MalformedArgs("hand_cap_body program"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #3515: Anointed's `CardPileCmd.Add` (`<Add>d__10` 0x3e1ba4 IL_0053)
    /// and the `CardSelectCmd.FromCombatPile` of Dredge and Neow's Fury
    /// (`<FromCombatPile>d__20` 0x3e5e84 IL_0036) return at `IsEnding`.
    /// While the combat is ending before the over latch no card moves and no
    /// selector opens; Anointed still rolls its shuffle (IL_0088 precedes
    /// the Add). The Adaptable-vetoed control moves or selects.
    #[test]
    fn hand_cap_moves_and_selectors_skip_while_combat_is_ending_before_the_over_latch() {
        let rare = plain(CardId::Scrawl, 0);
        let mut builder = CatalogBuilder::new();
        for owner in [
            plain(CardId::Anointed, 0),
            plain(CardId::Dredge, 0),
            plain(CardId::NeowsFury, 0),
        ] {
            builder.intern(owner).unwrap();
        }
        let rare_atom = builder.intern(rare).unwrap();
        let catalog = builder.build();
        for vetoed in [false, true] {
            let mut anointed = HotState::at_defaults();
            anointed.rng.set(
                RngStream::Sel,
                RngStreamState {
                    words: Xoshiro256StarStar::from_seed(17).words,
                    counter: 0,
                },
            );
            anointed
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend([card(1, rare_atom), card(4, rare_atom)]);
            crate::engine::damage::push_ending_window_roster(&mut anointed, vetoed);
            assert_eq!(
                begin(
                    &mut anointed,
                    &catalog,
                    plain(CardId::Anointed, 0),
                    None,
                    &mut Vec::new()
                )
                .unwrap(),
                None
            );
            assert_eq!(
                anointed.rng.get(RngStream::Sel).counter,
                1,
                "vetoed={vetoed}"
            );
            assert_eq!(
                anointed.piles.get(PileId::Hand).len(),
                2 * usize::from(vetoed)
            );

            let mut dredge = HotState::at_defaults();
            dredge
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(card(2, rare_atom));
            crate::engine::damage::push_ending_window_roster(&mut dredge, vetoed);
            begin(
                &mut dredge,
                &catalog,
                plain(CardId::Dredge, 0),
                None,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(dredge.piles.get(PileId::Hand).len(), usize::from(vetoed));

            let mut fury = HotState::at_defaults();
            fury.piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(card(3, rare_atom));
            let target = crate::engine::damage::push_ending_window_roster(&mut fury, vetoed);
            let selection = begin(
                &mut fury,
                &catalog,
                plain(CardId::NeowsFury, 0),
                Some(target),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(selection.is_some(), vetoed, "Neow's Fury vetoed={vetoed}");
        }
    }
}
