//! Card-step bodies for the `content/cards/status.py` family — GENERATED ONCE, THEN OWNED BY HAND.
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
//! # R2 status (#1560): 1 of 1 ported
//!
//! Frantic Escape's active-card identity and ordered local-cost write both
//! have Rust representations. Batch M admits its exact generated L0 row and
//! the complete optional The Insatiable/Sandpit owner lifecycle; other
//! playable Status/Curse rows remain refused independently.

use super::StepCtx;
use crate::engine::damage::note_power;
use crate::engine::{EngineRefusal, Subject};
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, LocalCostExpiration, LocalCostModifier,
    LocalCostModifierKind, PileId,
};
use crate::ids::{CardId, PowerId, StepKind};
use crate::powers::SlotWire;

/// The StepKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
pub const IMPLEMENTED: &[StepKind] = &[StepKind::FranticEscapeExact];

/// `frantic_escape_exact` — grow Sandpit, then this physical card's cost.
///
/// Python: `_run_steps_inner` (frozen, deleted #2827) and `_frantic_escape_exact`. The
/// body authenticates this exact active physical uid, appends an Add(+1),
/// ThisCombat local-cost row, validates the singleton The Insatiable roster,
/// and increments its live positive Sandpit before publishing the card write.
///
pub(crate) fn frantic_escape_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty()
        || ctx.target.is_some()
        || ctx.selection.is_some()
        || ctx.spec.identity.id != CardId::FranticEscape
        || ctx.spec.identity.upgrade != 0
        || crate::content_tables::card_row(CardId::FranticEscape, 0) != Some(ctx.spec.row)
    {
        return Err(EngineRefusal::MalformedArgs("frantic_escape_exact row"));
    }
    if !crate::engine::monsters::optional_insatiable_state_is_valid(ctx.state) {
        return Err(EngineRefusal::MalformedArgs(
            "The Insatiable/Sandpit lifecycle",
        ));
    }
    let expected_atom = ctx
        .catalog
        .atom(&ctx.spec.identity)
        .ok_or(EngineRefusal::UnknownMintIdentity(ctx.spec.identity))?;
    let active_index = ctx
        .state
        .piles
        .get(PileId::Play)
        .as_slice()
        .iter()
        .position(|card| card.uid == ctx.source_uid && card.atom == expected_atom)
        .ok_or(EngineRefusal::FrozenCardVanished {
            uid: ctx.source_uid,
            pile: PileId::Play,
        })?;

    let sandpit_update = ctx
        .state
        .monsters
        .iter()
        .position(|monster| monster.kind == crate::ids::MonsterKind::TheInsatiable)
        .and_then(|index| {
            let amount = ctx.state.monsters[index].powers.value(PowerId::Sandpit);
            (ctx.state.monsters[index].hp > 0 && amount > 0).then_some((index, amount))
        })
        .map(|(index, amount)| {
            amount
                .checked_add(1)
                .map(|updated| (index, updated))
                .ok_or(EngineRefusal::CounterOverflow("Sandpit"))
        })
        .transpose()?;

    if let Some((index, updated)) = sandpit_update {
        let uid = ctx.state.monsters[index].uid;
        ctx.state.monsters_mut()[index]
            .powers
            .set(PowerId::Sandpit, SlotWire::Int, updated);
        note_power(ctx.events, Subject::Monster(uid), PowerId::Sandpit, updated);
    }
    let active = &mut ctx.state.piles.get_mut(PileId::Play).make_mut()[active_index];
    active.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    let active = *active;
    ctx.state.card_states.append_local_cost_modifier(
        active.uid,
        LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: 1,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        },
    );
    let active_state = ctx.state.card_states.get_ref(active.uid);
    if PileId::ALL.iter().any(|pile| {
        ctx.state.piles.get(*pile).as_slice().iter().any(|card| {
            card.uid != active.uid
                && card.atom == active.atom
                && (card.flags != active.flags
                    || ctx.state.card_states.get_ref(card.uid) != active_state)
        })
    }) {
        ctx.state.exact_piles = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::engine::Event;
    use crate::hot::{HotCard, HotMonster, HotState};
    use crate::ids::MonsterKind;

    fn fixture(with_owner: bool) -> (HotState, crate::catalog::Catalog, HotCard) {
        let identity = CardIdentity {
            id: CardId::FranticEscape,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 17,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Play).make_mut().push(source);
        if with_owner {
            let mut owner = HotMonster::new(
                MonsterKind::TheInsatiable,
                crate::engine::monsters::THE_INSATIABLE_HP,
            );
            owner.max_hp = crate::engine::monsters::THE_INSATIABLE_HP;
            owner.loop_pos = 1;
            owner.powers.set(PowerId::Sandpit, SlotWire::Int, 4);
            state.monsters_mut().push(owner);
        }
        (state, catalog, source)
    }

    fn run(
        state: &mut HotState,
        catalog: &crate::catalog::Catalog,
        source: HotCard,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let spec = *catalog.spec(source.atom).unwrap();
        frantic_escape_exact(&mut StepCtx {
            state,
            catalog,
            spec: &spec,
            source_uid: source.uid,
            target: None,
            selection: None,
            x_value: 0,
            args: &[],
            events,
        })
    }

    #[test]
    fn frantic_escape_is_claimed_only_with_the_exact_body() {
        assert_eq!(IMPLEMENTED, [StepKind::FranticEscapeExact]);
    }

    #[test]
    fn frantic_escape_grows_sandpit_before_publishing_this_physical_cost() {
        let (mut state, catalog, source) = fixture(true);
        let mut events = Vec::new();

        run(&mut state, &catalog, source, &mut events).unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Sandpit), 5);
        assert!(matches!(
            events.first(),
            Some(Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Sandpit,
                amount: 5,
            })
        ));
        assert_ne!(
            state.piles.get(PileId::Play).as_slice()[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            0
        );
        assert_eq!(
            state
                .card_states
                .get(source.uid)
                .local_cost_modifiers
                .as_slice(),
            &[LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: 1,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        );

        run(&mut state, &catalog, source, &mut events).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Sandpit), 6);
        assert_eq!(
            state
                .card_states
                .get(source.uid)
                .local_cost_modifiers
                .as_slice()
                .len(),
            2
        );
    }

    #[test]
    fn frantic_escape_without_owner_is_cost_only_and_ties_force_exact_piles() {
        let (mut state, catalog, source) = fixture(false);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard { uid: 18, ..source });

        run(&mut state, &catalog, source, &mut Vec::new()).unwrap();

        assert!(state.monsters.is_empty());
        assert!(state.exact_piles);
        assert!(state.card_states.get(18).is_vacant());
    }

    #[test]
    fn frantic_escape_preflights_owner_overflow_and_source_identity_atomically() {
        let (mut state, catalog, source) = fixture(true);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Sandpit, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            run(&mut state, &catalog, source, &mut events),
            Err(EngineRefusal::CounterOverflow("Sandpit"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state.monsters_mut()[0]
            .powers
            .set(PowerId::Sandpit, SlotWire::Int, 4);
        state.piles.get_mut(PileId::Play).make_mut()[0].uid = 18;
        let before = state.clone();
        assert!(matches!(
            run(&mut state, &catalog, source, &mut events),
            Err(EngineRefusal::FrozenCardVanished {
                uid: 17,
                pile: PileId::Play
            })
        ));
        assert_eq!(state, before);
        assert!(events.is_empty());
    }
}
