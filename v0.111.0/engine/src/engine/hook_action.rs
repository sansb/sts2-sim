//! The deferred-choice hook-action queue (#3387): a choice begun inside a
//! listener of a deferred-choice `Hook` walk resolves in a queued
//! `GenericHookGameAction`, after the rest of the walk and of the enclosing
//! action.
//!
//! # Native
//!
//! v0.111.0, SHA-256 `9cb4f1ad…`. Six `Hook` walks give each listener its
//! own `HookPlayerChoiceContext` and await
//! `AssignTaskAndWaitForPauseOrCompletion` rather than the listener's task:
//! `<AfterDeath>d__28` `0x3cd984` (IL_008f, IL_00b6), `<AfterDiedToDoom>d__30`
//! `0x3cdb4c` (IL_007e, IL_0099), `<BeforeSideTurnStart>d__74` `0x3d39a8`
//! (IL_007e, IL_00a5), `<BeforeSideTurnEnd>d__80` `0x3d34f4` (IL_0093/00bd,
//! IL_01ab/01d8, IL_02c9/02f6; `WhenAll` IL_03a8), `<BeforeFlush>d__33`
//! `0x3d2acc` (IL_008f/00b3, IL_01a1/01c8; `WhenAll` IL_027a) and
//! `<AfterSideTurnEnd>d__81` `0x3d11c0` (IL_0093/00bd, `WhenAll` IL_016c;
//! IL_0219/0246, `WhenAll` IL_02f8). That await
//! (`<AssignTaskAndWaitForPauseOrCompletion>d__34` `0x3d62fc` IL_0053-0070)
//! is a `WhenAny` of the listener's task and the context's paused source.
//!
//! `HookPlayerChoiceContext/<SignalPlayerChoiceBegun>d__37` `0x3d64f4`, on a
//! listener's first choice: `GenerateHookAction` (IL_01b1-01ce), wait for the
//! listener's task to be assigned (IL_01de-0244), `SetChoiceContext`
//! (IL_025c-0263), and for the local player (IL_0268-0279)
//! `RequestEnqueueHookAction` (IL_0282-028e), release the paused source so the
//! walk moves on (IL_0293-0299), wait for the hook action to start
//! (`ExecutionStartedTask`, IL_029e-02f9), and only then
//! `PauseActionForPlayerChoice` (IL_02fe-0310). A later choice of the same
//! listener pauses its already-running hook action directly (IL_00e7-010b),
//! so it is sequential.
//!
//! `ActionQueueSynchronizer.RequestEnqueueHookAction` `0x1105b0` enqueues
//! locally (IL_00b7-00be, `EnqueueHookAction` `0x110840` IL_00e9-00f0), and
//! `ActionQueueSet.EnqueueWithoutSynchronizing` `0x10e970` stamps the next
//! action id (IL_0091-00a4) and appends it to the BACK of the owner's queue
//! (IL_02da-02e1). `GetReadyAction` `0x10ec90` takes the lowest-id front
//! action (IL_036d-041b), and `ActionExecutor/<ExecuteActions>d__28`
//! `0x3d4310` runs one action at a time: it awaits the running action
//! (IL_0143-0177), runs `CheckWinCondition` (IL_0334) and only then takes the
//! next ready action (IL_0472). `GenericHookGameAction/<ExecuteAction>d__`
//! `0x3d50d0` releases `ExecutionStarted` (IL_008d-0092) and awaits the
//! listener's task (IL_0098-009d).
//!
//! The option list is read once the hook action has started:
//! `CardSelectCmd/<FromCombatPile>d__20` `0x3e5e84` awaits the signal at
//! IL_00ae and reads `pile.Cards` at IL_0114-0141, and `<FromHand>d__28`
//! `0x3e7568` signals at IL_00b3 and reads the Hand at IL_010f-014c.
//!
//! So a choice begun inside a player action (a PlayCardAction or a potion's
//! action) resolves after the whole action has finished, in enqueue order,
//! and sees the piles as the action left them. A death during EndTurn (turn
//! end, enemy turn, turn start) is different: that flow is `CombatManager`'s
//! own, not a queued action, and nothing in it waits on the queue
//! (`FinishedExecutingActions` is awaited only by `<StartCombatInternal>d__98`
//! `0x3f71b0` and `RunReplay`), so the hook action would run on the idle
//! executor concurrently with it. That is refused by name.
//!
//! # Listeners that can begin a choice
//!
//! An override census of every `<Hook>d__N::MoveNext` body of the three walks
//! without a side-turn `WhenAll` finds one: `GremlinHorn/<AfterDeath>d__6`
//! `0x326170`, which awaits `GainEnergy` (IL_0062) and then
//! `CardPileCmd.Draw(choiceContext, Cards, Owner, false)` (IL_00d9) as its
//! tail (`leave` IL_0131). No AfterDiedToDoom, BeforeFlush or
//! BeforeSideTurnStart listener reaches a player choice. The side-turn walks'
//! choosing listeners stay under `puzzle::DeferredChoiceListener` (#3386),
//! which counts their unprompted selects with the same scope as the Horn's
//! Draw and refuses them by name under a named wall (#3485).
//!
//! # Model
//!
//! Gremlin Horn's Draw runs on the resumable Draw frame with
//! [`DrawCaller::GremlinHorn`] whenever Hellraiser or Stratagem is live. If
//! it suspends inside a player action, the Draw frame, its children and the
//! pending choice are detached into the state's transient queue
//! ([`HotState::queue_deferred_hook_draw`]) and the listener returns, so the
//! walk and the enclosing action run on. When the public transaction has
//! driven its frames back to its base, [`publish_queued`] re-attaches the
//! segment on top and publishes it as an ordinary parked Draw, whose
//! ActionReplay root is the public action. Answers resume its frames
//! directly. The queue itself is never serialized.
//!
//! A Hellraiser AutoPlay under the Draw keeps its card in Play while the
//! enclosing action finishes, because native routes a finishing card by
//! reference (`play::take_manual_play_instance`). The queued action pins the
//! Play pile to exactly those suspended cards, as it pins the Hand and Draw
//! piles its option lists read. A select command that resolves without a
//! prompt still signals natively, so Rust refuses it by name
//! ([`AUTO_RESOLVED_CHOICE`]).
//!
//! # Not modeled (each refuses by name)
//!
//! * **A queue that must cross the boundary** ([`SECOND_HOOK_ACTION`],
//!   [`BESIDE_A_CHOICE`]). A second hook action queued behind the first, or
//!   the enclosing action's own choice resolving while a hook action waits,
//!   parks a state that still owns a continuation that is not on its stack.
//!   `CanonicalStateV2` has no carrier for a detached segment. The import
//!   authenticator re-executes only the root, so it cannot rebuild a queue
//!   that forms after the first answer. Carrying the queue would change the
//!   boundary format.
//! * **EndTurn deaths** ([`OUTSIDE_PLAYER_ACTION`]). The EndTurn flow is
//!   `CombatManager`'s own, and the hook action would run concurrently with
//!   it (see above).
//!
//! [`DrawCaller::GremlinHorn`]: crate::hot::DrawCaller::GremlinHorn
//! [`HotState::queue_deferred_hook_draw`]: crate::hot::HotState::queue_deferred_hook_draw

use std::cell::Cell;

use crate::catalog::Catalog;
use crate::hot::{DrawCaller, HotState};

use super::{EngineRefusal, Event};

/// A choice from a Gremlin Horn Draw during EndTurn (see the module docs).
pub(crate) const OUTSIDE_PLAYER_ACTION: &str = "Gremlin Horn Draw choice outside a player action";
/// A choice from a Gremlin Horn Draw on the rootless legacy path, which
/// cannot publish the deferred Draw's ActionReplay root.
pub(crate) const WITHOUT_RECEIPT: &str =
    "Gremlin Horn Draw choice without an action replay receipt";
/// A choice from a Gremlin Horn Draw while a receipt re-executes its action.
pub(crate) const INSIDE_RECEIPT_REEXECUTION: &str =
    "Gremlin Horn Draw choice inside a receipt re-execution";
/// A second hook action queued behind the first.
pub(crate) const SECOND_HOOK_ACTION: &str = "second deferred hook action";
/// The enclosing action begins its own choice while a hook action is queued.
pub(crate) const BESIDE_A_CHOICE: &str = "choice beside a deferred hook action";
/// The enclosing action ends the combat while a hook action is queued.
pub(crate) const AT_COMBAT_END: &str = "deferred hook action at combat end";
/// The enclosing action requests Void Form's end of turn.
pub(crate) const BESIDE_VOID_FORM_END_TURN: &str =
    "deferred hook action beside a Void Form end-turn request";
/// The Hand or Draw pile changed between the choice beginning and the hook
/// action starting.
pub(crate) const CHOICE_SOURCE_MOVED: &str = "deferred hook action choice source moved";
/// A deferred choice whose option list has not been audited.
pub(crate) const UNAUDITED_CHOICE: &str = "unaudited deferred hook action choice";
/// A deferred choice above a suspended card play that is not a Hellraiser
/// AutoPlay of the Draw's own card (the only card play the Draw child
/// grammar nests, `play::rooted_draw_child_suffix_is_exact`).
pub(crate) const UNMODELED_CARD_PLAY: &str = "deferred hook action above an unmodeled card play";
/// The Play pile, once the enclosing action has finished, is not exactly the
/// queued hook action's own suspended card plays as they were when its
/// choice began.
///
/// Native keeps a suspended AutoPlayed card in Play while the enclosing
/// action finishes, and routes the enclosing card by reference
/// (`play::take_manual_play_instance`). Anything else touching Play in
/// between is not audited.
pub(crate) const PLAY_PILE_MOVED: &str = "deferred hook action Play pile moved";
/// A select command inside the listener that resolved without a prompt.
///
/// Native signals the listener's context before reading the pile and before
/// deciding whether a prompt is needed: `CardSelectCmd/<FromCombatPile>d__20`
/// RVA `0x3e5e84` signals at IL_00a2-00ae, reads `pile.Cards` at
/// IL_010c-013b and auto-takes at IL_0140-017c; `<FromHand>d__28` RVA
/// `0x3e7568` signals at IL_00a7-00b3, reads at IL_010d-014c and auto-takes
/// at IL_0152-018d. Only `IsEnding`/`IsOverOrEnding` (IL_0036) or a set
/// `Selector` (IL_0077-007c / IL_007c-0081) skips the signal. So even an
/// auto-resolved selection queues the rest of the listener as a hook
/// action, behind the enclosing action. Rust resolves it inline, so it
/// refuses.
pub(crate) const AUTO_RESOLVED_CHOICE: &str =
    "deferred hook action choice resolved without a prompt";

/// Which kind of public transaction is running (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionScope {
    /// No replay-capable transaction (the legacy path or a bare test call).
    Rootless,
    /// A PlayCardAction or a potion's action, or a Select resuming one.
    PlayerAction,
    /// EndTurn, or a Select resuming one.
    OtherAction,
}

thread_local! {
    static SCOPE: Cell<ActionScope> = const { Cell::new(ActionScope::Rootless) };
}

/// The running transaction's scope, restored on drop.
pub(crate) struct ScopeGuard(ActionScope);

impl ScopeGuard {
    pub(crate) fn enter(scope: ActionScope) -> Self {
        Self(SCOPE.with(|cell| cell.replace(scope)))
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPE.with(|cell| cell.set(self.0));
    }
}

fn scope() -> ActionScope {
    SCOPE.with(Cell::get)
}

thread_local! {
    /// Select commands that signalled the innermost unsignalled
    /// deferred-choice listener without prompting (`None`: no such listener
    /// is running). See [`AUTO_RESOLVED_CHOICE`].
    static UNPROMPTED_SIGNALS: Cell<Option<u32>> = const { Cell::new(None) };
}

/// A deferred-choice listener that has not yet signalled its context, for
/// the duration of its Draw (#3387), or of a side-turn / death walk's
/// listeners (`puzzle::DeferredChoiceListener`, #3485). Nested listeners (a
/// death inside the Draw runs its own `AfterDeath` walk, and so its own
/// context) save and restore the outer count.
pub(crate) struct UnsignaledListener(Option<u32>);

impl UnsignaledListener {
    /// Count the unprompted selects of a listener whose deferral Rust does
    /// not model.
    pub(crate) fn enter() -> Self {
        Self(UNPROMPTED_SIGNALS.with(|cell| cell.replace(Some(0))))
    }

    /// A listener whose later peers are proven to commute with a choice and
    /// its continuation (`puzzle::DeferredChoiceListener::enter(None)`):
    /// resolving inline is exact, so its selects are not counted, and they
    /// are not charged to an enclosing listener either (the choice signals
    /// this listener's own context).
    pub(crate) fn exempt() -> Self {
        Self(UNPROMPTED_SIGNALS.with(|cell| cell.replace(None)))
    }

    pub(crate) fn unprompted_signals(&self) -> u32 {
        UNPROMPTED_SIGNALS.with(Cell::get).unwrap_or(0)
    }
}

impl Drop for UnsignaledListener {
    fn drop(&mut self) {
        UNPROMPTED_SIGNALS.with(|cell| cell.set(self.0));
    }
}

/// Record that a native `CardSelectCmd` has just resolved without a prompt
/// on the running listener's context (#3387, [`AUTO_RESOLVED_CHOICE`]).
/// Callers are the Rust arms that take the option list directly:
/// `draw::stratagem_after_shuffle`, Seeker Strike's short-list arm, and
/// `selection::execute`'s auto-take. Each passes whether native would
/// signal at all: the combat is not ending, and no `Selector` is set.
/// Outside a counted deferred-choice listener this is a no-op.
pub(crate) fn note_unprompted_select(signals: bool) {
    if signals {
        UNPROMPTED_SIGNALS.with(|cell| {
            if let Some(count) = cell.get() {
                cell.set(Some(count.saturating_add(1)));
            }
        });
    }
}

/// The scope of a replay-capable transaction applying `action`, on a state
/// whose ActionReplay root (if any) names `root`: a Play or a UsePotion is a
/// queued player action, and so is a Select that resumes one. Everything
/// else is EndTurn's.
pub(crate) fn scope_for(
    action: &super::Action,
    root: Option<crate::hot::ActionReplayRootAction>,
) -> ActionScope {
    use crate::hot::ActionReplayRootAction;
    match (action, root) {
        (super::Action::Play { .. } | super::Action::UsePotion { .. }, _)
        | (
            super::Action::Select { .. },
            Some(ActionReplayRootAction::Play { .. } | ActionReplayRootAction::UsePotion { .. }),
        ) => ActionScope::PlayerAction,
        _ => ActionScope::OtherAction,
    }
}

/// Whether the deferred choice's option list was audited: native reads it
/// only once the hook action has started, from a pile [`publish_queued`]
/// pins (the Hand and the Draw pile, with each card's instance state).
///
/// * Stratagem's `AfterShuffle` selection: `StratagemPower/<AfterShuffle>d__4`
///   RVA `0x34688c` awaits `CardSelectCmd.FromCombatPile` over the owner's
///   Draw pile (IL_0045-0078), which reads `pile.Cards` only after the
///   signal (`<FromCombatPile>d__20` `0x3e5e84` IL_00ae, then
///   IL_010c-013b).
/// * A Hellraiser-AutoPlayed Seeker Strike's selection:
///   `SeekerStrike/<OnPlay>d__5::MoveNext` RVA `0x3b9754` shuffles the Draw
///   pile's cards with `CombatCardSelection` into `cardOptions`
///   (IL_00ed-0138), BEFORE the signal, and awaits
///   `FromCombatPile(Draw, filter: cardOptions.Contains)` (IL_013d-0172). The
///   RNG is spent in the listener's prefix in both engines. Once the hook
///   action starts, the options are the live Draw pile filtered by the
///   short list, and the answer moves the pick into the Hand (whose cap
///   reads the Hand).
/// * A Hellraiser-AutoPlayed Sculpting Strike's selection:
///   `SculptingStrike/<OnPlay>d__7::MoveNext` RVA `0x3b8cec` awaits
///   `FromHand` with its Ethereal filter (IL_00de-0115). `<FromHand>d__28`
///   `0x3e7568` signals at IL_00b3 and reads the Hand at IL_010d-014c. The
///   filter reads the card's local keywords (instance state).
///
/// Hellraiser passes the walk's context to the AutoPlay:
/// `HellraiserPower/<AfterCardDrawnEarly>d__7::MoveNext` RVA `0x33c1a8`
/// calls `CardCmd.AutoPlay(choiceContext, card, …)` at IL_011e-012e. So a
/// Strike's selection signals the Horn listener's context.
fn deferred_choice_is_audited(state: &HotState, catalog: &Catalog) -> bool {
    use crate::hot::{CardPlaySource, PendingSelectionKind};
    use crate::ids::CardId;
    let Some(pending) = state.pending.as_deref() else {
        return false;
    };
    if pending.stratagem_draw_record(&state.frames).is_some() {
        return true;
    }
    state
        .pending_card_play(pending)
        .and_then(|(play, card)| Some((play, catalog.spec(card.atom)?)))
        .is_some_and(|(play, spec)| {
            play.source == CardPlaySource::Hellraiser
                && play.pending_choice
                && matches!(
                    (spec.identity.id, play.selection_kind),
                    (
                        CardId::SeekerStrike,
                        Some(PendingSelectionKind::SeekerStrike)
                    ) | (CardId::SculptingStrike, Some(PendingSelectionKind::Program))
                )
        })
}

/// Whether every card play in the suspended segment above `depth` is a
/// Hellraiser AutoPlay: the one card play the Draw child grammar nests.
/// Native leaves that card in the Play pile while the enclosing action
/// finishes.
fn segment_card_plays_are_hellraiser(state: &HotState, depth: usize) -> bool {
    state.frames.as_slice()[depth..]
        .iter()
        .all(|frame| match *frame {
            crate::frame::Frame::CardPlay { record } => state
                .frames
                .card_play(record)
                .is_some_and(|play| play.source == crate::hot::CardPlaySource::Hellraiser),
            _ => true,
        })
}

/// Issue one deferred-choice listener's `CardPileCmd.Draw` of `n` cards on
/// the resumable frame (#3387). If it completes, the listener continues; if
/// it begins a choice, the choice is queued as a hook action and the
/// listener returns (see the module docs). A select command that resolves
/// without a prompt still signals natively, so it refuses
/// ([`AUTO_RESOLVED_CHOICE`]).
pub(crate) fn deferred_listener_draw(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if caller != DrawCaller::GremlinHorn {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let depth = state.frames.len();
    let word_base = state.frames.word_len();
    let listener = UnsignaledListener::enter();
    let drawn = super::draw::draw_cards_for_potion(state, catalog, n, caller, events)?;
    if listener.unprompted_signals() > 0 {
        return Err(EngineRefusal::PowerOrderNotModeled(AUTO_RESOLVED_CHOICE));
    }
    drop(listener);
    match drawn {
        super::draw::PotionDrawResult::Complete => return Ok(()),
        super::draw::PotionDrawResult::Suspended => {}
    }
    if state.pending.is_none() || state.frames.len() <= depth {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    match scope() {
        ActionScope::PlayerAction => {}
        ActionScope::OtherAction => {
            return Err(EngineRefusal::PowerOrderNotModeled(OUTSIDE_PLAYER_ACTION));
        }
        ActionScope::Rootless => return Err(EngineRefusal::PowerOrderNotModeled(WITHOUT_RECEIPT)),
    }
    if super::puzzle::tape_is_installed() {
        return Err(EngineRefusal::PowerOrderNotModeled(
            INSIDE_RECEIPT_REEXECUTION,
        ));
    }
    if state.fanouts.deferred_hook_action_is_queued() {
        return Err(EngineRefusal::PowerOrderNotModeled(SECOND_HOOK_ACTION));
    }
    if !deferred_choice_is_audited(state, catalog) {
        return Err(EngineRefusal::PowerOrderNotModeled(UNAUDITED_CHOICE));
    }
    if !segment_card_plays_are_hellraiser(state, depth) {
        return Err(EngineRefusal::PowerOrderNotModeled(UNMODELED_CARD_PLAY));
    }
    #[cfg(test)]
    hit(|hits| hits.queued += 1);
    state
        .queue_deferred_hook_draw(depth, word_base)
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

/// Publish the queued hook action once the public transaction's own frames
/// have drained to its base (#3387): the executor's next ready action.
/// Returns whether a hook action was published.
pub(crate) fn publish_queued(state: &mut HotState) -> Result<bool, EngineRefusal> {
    if !state.fanouts.deferred_hook_action_is_queued() {
        return Ok(false);
    }
    // The action's own choice resolves first, with the hook action still
    // queued behind it: that queue would have to cross the boundary.
    if state.pending.is_some() {
        return Err(EngineRefusal::PowerOrderNotModeled(BESIDE_A_CHOICE));
    }
    if !(state.frames.is_empty()
        || matches!(
            state.frames.as_slice(),
            [crate::frame::Frame::ActionReplay { .. }]
        ))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // `CheckWinCondition` (`0x3d4310` IL_0334) runs before the next ready
    // action is taken; what an ending combat does with a queued hook action
    // is not modeled.
    if state.history.over {
        return Err(EngineRefusal::PowerOrderNotModeled(AT_COMBAT_END));
    }
    if state.fanouts.void_form_end_turn_requested() {
        return Err(EngineRefusal::PowerOrderNotModeled(
            BESIDE_VOID_FORM_END_TURN,
        ));
    }
    if !state.deferred_hook_choice_piles_are_unmoved() {
        return Err(EngineRefusal::PowerOrderNotModeled(CHOICE_SOURCE_MOVED));
    }
    if !state.deferred_hook_play_pile_is_the_segments() {
        return Err(EngineRefusal::PowerOrderNotModeled(PLAY_PILE_MOVED));
    }
    state
        .publish_deferred_hook_draw()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    #[cfg(test)]
    hit(|hits| hits.published += 1);
    Ok(true)
}

/// Test-only branch witnesses (#2700).
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BranchHits {
    /// Choices queued as a deferred hook action.
    pub queued: u32,
    /// Queued hook actions published at the end of their transaction.
    pub published: u32,
}

#[cfg(test)]
thread_local! {
    static BRANCH_HITS: Cell<BranchHits> = const {
        Cell::new(BranchHits { queued: 0, published: 0 })
    };
}

#[cfg(test)]
pub(crate) fn take_branch_hits() -> BranchHits {
    BRANCH_HITS.with(|hits| hits.replace(BranchHits::default()))
}

#[cfg(test)]
fn hit(update: impl FnOnce(&mut BranchHits)) {
    BRANCH_HITS.with(|hits| {
        let mut value = hits.get();
        update(&mut value);
        hits.set(value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::catalog::{CardAtom, CardIdentity, CatalogBuilder};
    use crate::engine::admission::{PHASE_ORDINARY_ACTIONS, admit};
    use crate::engine::{Action, SelectionRef, apply_action, legal_actions};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, PileId};
    use crate::ids::CardId;
    use crate::ids::{MonsterKind, PowerId, RelicId};
    use crate::powers::SlotWire;

    fn card(uid: u32, atom: CardAtom) -> HotCard {
        HotCard {
            uid,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        }
    }

    /// Canonical round trip plus admission: every published state must load.
    fn cold(state: &HotState, catalog: &Catalog) -> (HotState, Catalog) {
        let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let loaded = HotBoundary::from_canonical(&wire, &loaded_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&loaded, &loaded_catalog).unwrap(),
            wire
        );
        assert_eq!(admit(&wire, &loaded, &loaded_catalog), Ok(()));
        (loaded, loaded_catalog)
    }

    /// Drive every legal answer from `parked` to a state with no pending
    /// choice, round-tripping and admitting every parked state on the way.
    fn every_answer_to_terminal(parked: &HotState, catalog: &Catalog) -> (Vec<HotState>, usize) {
        let mut work = vec![parked.clone()];
        let mut terminals = Vec::new();
        let mut visited = 0;
        while let Some(state) = work.pop() {
            visited += 1;
            assert!(visited < 10_000, "unbounded answer tree");
            let (loaded, loaded_catalog) = cold(&state, catalog);
            assert_eq!(loaded, state);
            let actions = legal_actions(&loaded, &loaded_catalog);
            assert!(!actions.is_empty());
            for action in actions {
                let next = apply_action(&loaded, &loaded_catalog, &action)
                    .unwrap_or_else(|error| panic!("{action:?}: {error:?}"))
                    .state;
                if next.pending.is_some() {
                    work.push(next);
                } else {
                    assert!(next.frames.is_empty());
                    cold(&next, catalog);
                    terminals.push(next);
                }
            }
        }
        (terminals, visited)
    }

    struct Fixture {
        state: HotState,
        catalog: Catalog,
        atoms: Vec<CardAtom>,
    }

    /// A Toadpole fight holding Gremlin Horn (and `extra` relics), with one
    /// Toadpole per `hps` entry (uids and slots in roster order) and `cards`
    /// interned in order.
    fn fixture(cards: &[CardId], extra: &[RelicId], hps: &[i32]) -> Fixture {
        let mut builder = CatalogBuilder::new();
        let atoms = cards
            .iter()
            .map(|id| {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap()
            })
            .collect();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let mut relics = vec![RelicId::RelicGremlinHorn];
        relics.extend_from_slice(extra);
        builder.set_relics_ordered(&relics, false).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 20;
        state.exact_piles = true;
        state.fanouts.set_gremlin_horn_owned(true);
        for stream in [
            crate::hot::RngStream::Rng,
            crate::hot::RngStream::Sel,
            crate::hot::RngStream::Targets,
        ] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        let monsters = hps
            .iter()
            .enumerate()
            .map(|(index, &hp)| {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
                monster.max_hp = hp;
                monster.uid = index as u32;
                monster.slot = index as i32;
                monster
            })
            .collect();
        state.monsters = std::sync::Arc::new(monsters);
        Fixture {
            state,
            catalog,
            atoms,
        }
    }

    fn push(fixture: &mut Fixture, pile: PileId, cards: impl IntoIterator<Item = HotCard>) {
        fixture.state.piles.get_mut(pile).make_mut().extend(cards);
    }

    const STRIKE_THE_WEAK_TOADPOLE: Action = Action::Play {
        uid: 1,
        target: Some(0),
        selection: SelectionRef::NONE,
    };

    /// Strike (6) in Hand kills a 5-HP Toadpole beside a 1,000-HP one; the
    /// Draw pile is empty and five Defends sit in the Discard pile, so the
    /// Horn's one-card Draw reshuffles five cards against Stratagem 2.
    fn stratagem_kill() -> Fixture {
        let mut fixture = fixture(
            &[CardId::StrikeIronclad, CardId::DefendIronclad],
            &[],
            &[5, 1_000],
        );
        let (strike, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        push(&mut fixture, PileId::Hand, [card(1, strike)]);
        push(
            &mut fixture,
            PileId::Discard,
            (2..7).map(|uid| card(uid, defend)),
        );
        fixture
    }

    /// The published hook action's Draw record, asserting its stack shape.
    fn published_horn_draw(parked: &HotState) -> crate::hot::DrawRecord {
        assert!(parked.pending.is_some());
        assert!(!parked.fanouts.deferred_hook_action_is_queued());
        let [
            crate::frame::Frame::ActionReplay { .. },
            crate::frame::Frame::Draw { record },
            ..,
        ] = parked.frames.as_slice()
        else {
            panic!("{:#?}", parked.frames);
        };
        let draw = parked.frames.draw(*record).unwrap().to_owned();
        assert_eq!(draw.caller, DrawCaller::GremlinHorn);
        assert_eq!(draw.requested, 1);
        draw
    }

    /// #3387 AfterDeath witness, Stratagem: the Horn's Draw reshuffles and
    /// Stratagem's selection begins inside the AfterDeath listener. Native
    /// queues the rest of the Draw as a hook action, so the Strike's play
    /// finishes BEFORE the choice appears; the parked document is an ordinary
    /// Draw rooted at the Play, and every answer then finishes the Draw once.
    #[test]
    fn stratagem_choice_in_a_horn_draw_waits_for_the_play_to_finish() {
        let fixture = stratagem_kill();
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert!(catalog.requires_action_replay());
        take_branch_hits();
        let parked = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert_eq!(
            take_branch_hits(),
            BranchHits {
                queued: 1,
                published: 1
            }
        );
        let draw = published_horn_draw(&parked);
        assert_eq!(draw.completed, 0);
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        // The ordering witness: the enclosing play finished first.
        assert_eq!(parked.history.card_plays_finished_combat, 1);
        assert!(
            parked
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1),
            "the Strike reached the Discard pile before the choice"
        );
        assert!(parked.monsters[0].hp <= 0);
        // GainEnergy (IL_0062) ran in the listener's prefix.
        assert_eq!(parked.energy, state.energy - 1 + 1);
        assert_eq!(parked.piles.get(PileId::Draw).len(), 5);
        // Boundary round trip of the parked hook action.
        cold(&parked, &catalog);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20); // P(5, 2) ordered picks.
        let mut hands = std::collections::BTreeSet::new();
        for terminal in &terminals {
            // Two picked by Stratagem, then the Horn's one card.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 3);
            assert_eq!(terminal.cards_drawn_combat, 1);
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
            assert_eq!(terminal.energy, state.energy);
            assert!(terminal.frames.is_empty());
            hands.insert(
                terminal
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
            );
        }
        assert!(hands.len() > 1, "the answer must be observable");
    }

    /// Hellraiser 1, the Strike in Hand, and `top` on top of a Draw pile of
    /// Defends (none when `defends` is false), with `discard` Defends in the
    /// Discard pile.
    fn hellraiser_kill(top: CardId, defends: bool, discard: u32, stratagem: i32) -> Fixture {
        let mut fixture = fixture(
            &[CardId::StrikeIronclad, top, CardId::DefendIronclad],
            &[],
            &[5, 1_000],
        );
        let (strike, top, defend) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        if stratagem > 0 {
            fixture
                .state
                .powers
                .set(PowerId::Stratagem, SlotWire::Int, stratagem);
        }
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        push(&mut fixture, PileId::Hand, [card(1, strike)]);
        push(&mut fixture, PileId::Draw, [card(2, top)]);
        if defends {
            push(
                &mut fixture,
                PileId::Draw,
                (10..15).map(|uid| card(uid, defend)),
            );
        }
        push(
            &mut fixture,
            PileId::Discard,
            (15..15 + discard).map(|uid| card(uid, defend)),
        );
        fixture
    }

    fn uids(state: &HotState, pile: PileId) -> Vec<u32> {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect()
    }

    /// Strike the weak Toadpole in `fixture` and return the parked hook
    /// action. Asserts that it was queued and published once, that the
    /// Strike finished (it is in the Discard pile) while the Horn card the
    /// Hellraiser AutoPlayed (uid 2) is still in Play, and that the parked
    /// state round-trips.
    fn park_hellraiser_choice(fixture: &Fixture) -> (HotState, Catalog) {
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert!(catalog.requires_action_replay());
        take_branch_hits();
        let parked = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert_eq!(
            take_branch_hits(),
            BranchHits {
                queued: 1,
                published: 1
            }
        );
        let draw = published_horn_draw(&parked);
        assert_eq!(draw.card_uid, Some(2));
        // The ordering witness: the enclosing Strike finished by reference
        // beside the suspended AutoPlay, which is still in Play.
        assert_eq!(parked.history.card_plays_finished_combat, 1);
        assert_eq!(uids(&parked, PileId::Play), vec![2]);
        assert!(uids(&parked, PileId::Discard).contains(&1));
        assert!(parked.monsters[0].hp <= 0);
        let [
            crate::frame::Frame::ActionReplay { .. },
            crate::frame::Frame::Draw { .. },
            crate::frame::Frame::CardPlay { record },
            ..,
        ] = parked.frames.as_slice()
        else {
            panic!("{:#?}", parked.frames);
        };
        let child = parked.frames.card_play(*record).unwrap();
        assert_eq!(child.uid, 2);
        assert_eq!(child.source, crate::hot::CardPlaySource::Hellraiser);
        cold(&parked, &catalog);
        (parked, catalog)
    }

    /// #3387 gap 1, Seeker Strike: the Horn draws a Seeker, which Hellraiser
    /// AutoPlays, and the Seeker's short-list selection is the choice. It is
    /// deferred behind the Strike. Once the Strike has finished, every
    /// answer moves one short-listed Defend into the Hand and then finishes
    /// the Seeker after the Strike.
    #[test]
    fn a_hellraiser_seeker_choice_in_a_horn_draw_waits_for_the_play_to_finish() {
        let fixture = hellraiser_kill(CardId::SeekerStrike, true, 0, 0);
        let (parked, catalog) = park_hellraiser_choice(&fixture);
        assert!(deferred_choice_is_audited(&parked, &catalog));
        assert_eq!(parked.piles.get(PileId::Draw).len(), 5);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 3);
        let mut picks = std::collections::BTreeSet::new();
        for terminal in &terminals {
            let hand = uids(terminal, PileId::Hand);
            assert_eq!(hand.len(), 1);
            assert!((10..15).contains(&hand[0]));
            picks.insert(hand[0]);
            assert_eq!(uids(terminal, PileId::Discard), vec![1, 2]);
            assert!(terminal.piles.get(PileId::Play).is_empty());
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
            assert_eq!(terminal.piles.get(PileId::Draw).len(), 4);
        }
        assert_eq!(picks.len(), 3, "the answer must be observable");
    }

    /// #3387 gap 1, Sculpting Strike: the AutoPlayed Sculpting's Hand
    /// selection is deferred behind the Strike. Every answer changes exactly
    /// the picked Hand card (it gains Ethereal).
    #[test]
    fn a_hellraiser_sculpting_choice_in_a_horn_draw_waits_for_the_play_to_finish() {
        let mut fixture = hellraiser_kill(CardId::SculptingStrike, true, 0, 0);
        let defend = fixture.atoms[2];
        push(
            &mut fixture,
            PileId::Hand,
            (30..32).map(|uid| card(uid, defend)),
        );
        let (parked, catalog) = park_hellraiser_choice(&fixture);
        assert!(deferred_choice_is_audited(&parked, &catalog));
        assert_eq!(uids(&parked, PileId::Hand), vec![30, 31]);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 2);
        let mut marked = std::collections::BTreeSet::new();
        for terminal in &terminals {
            assert_eq!(uids(terminal, PileId::Hand), vec![30, 31]);
            assert_eq!(uids(terminal, PileId::Discard), vec![1, 2]);
            let changed = [30, 31]
                .into_iter()
                .filter(|&uid| {
                    let before = parked
                        .piles
                        .get(PileId::Hand)
                        .as_slice()
                        .iter()
                        .find(|card| card.uid == uid)
                        .map(|card| (*card, parked.card_states.get(uid)));
                    let after = terminal
                        .piles
                        .get(PileId::Hand)
                        .as_slice()
                        .iter()
                        .find(|card| card.uid == uid)
                        .map(|card| (*card, terminal.card_states.get(uid)));
                    before != after
                })
                .collect::<Vec<_>>();
            assert_eq!(changed.len(), 1, "exactly the pick changed");
            marked.insert(changed[0]);
        }
        assert_eq!(marked.len(), 2);
    }

    /// #3387 gap 1, Pommel Strike: the AutoPlayed Pommel's own Draw
    /// reshuffles against Stratagem, so the selection sits above the Pommel's
    /// card play. It is deferred behind the Strike. The Strike reaches the
    /// Discard pile only after the reshuffle, so it is not in the Draw pile.
    #[test]
    fn a_stratagem_choice_above_a_hellraiser_pommel_waits_for_the_play_to_finish() {
        let fixture = hellraiser_kill(CardId::PommelStrike, false, 5, 2);
        let (parked, catalog) = park_hellraiser_choice(&fixture);
        assert!(deferred_choice_is_audited(&parked, &catalog));
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        assert_eq!(uids(&parked, PileId::Discard), vec![1]);
        assert_eq!(parked.piles.get(PileId::Draw).len(), 5);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20); // P(5, 2)
        for terminal in &terminals {
            // Two Stratagem picks, then the Pommel's own one card.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 3);
            assert_eq!(uids(terminal, PileId::Discard), vec![1, 2]);
            assert!(terminal.piles.get(PileId::Play).is_empty());
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
        }
    }

    /// #3387 gap 1, auto-resolved selections. Native signals the Horn
    /// listener's context even when a select command takes its option list
    /// without a prompt, and that defers the rest of the listener. Rust
    /// resolves it inline, so each case refuses by name:
    /// - a Seeker whose short list is empty;
    /// - a Sculpting with one Hand candidate;
    /// - a reshuffle that leaves no more cards than Stratagem's Amount.
    #[test]
    fn an_auto_resolved_choice_in_a_horn_draw_refuses_by_name() {
        let seeker = hellraiser_kill(CardId::SeekerStrike, false, 0, 0);
        let mut sculpting = hellraiser_kill(CardId::SculptingStrike, true, 0, 0);
        let defend = sculpting.atoms[2];
        push(&mut sculpting, PileId::Hand, [card(30, defend)]);
        let mut stratagem = stratagem_kill();
        stratagem
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .truncate(2);
        for fixture in [seeker, sculpting, stratagem] {
            let (state, catalog) = cold(&fixture.state, &fixture.catalog);
            take_branch_hits();
            assert_eq!(
                apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE).map(|_| ()),
                Err(EngineRefusal::PowerOrderNotModeled(AUTO_RESOLVED_CHOICE))
            );
            assert_eq!(take_branch_hits(), BranchHits::default());
        }
    }

    /// #3387: the unsignaled-listener scope. Outside a listener, a note is a
    /// no-op. A note that native would not signal (ending, or a `Selector`
    /// set) is not counted. A nested listener restores the outer count.
    #[test]
    fn unprompted_select_notes_count_only_inside_the_innermost_listener() {
        note_unprompted_select(true);
        assert_eq!(UNPROMPTED_SIGNALS.with(Cell::get), None);
        let listener = UnsignaledListener::enter();
        note_unprompted_select(false);
        assert_eq!(listener.unprompted_signals(), 0);
        {
            let inner = UnsignaledListener::enter();
            note_unprompted_select(true);
            assert_eq!(inner.unprompted_signals(), 1);
        }
        assert_eq!(
            listener.unprompted_signals(),
            0,
            "a nested listener restores"
        );
        note_unprompted_select(true);
        assert_eq!(listener.unprompted_signals(), 1);
        drop(listener);
        assert_eq!(UNPROMPTED_SIGNALS.with(Cell::get), None);
    }

    /// #3387 non-choice witness under Hellraiser: the Horn draws a plain
    /// Strike, which Hellraiser AutoPlays inline. Nothing is queued and no
    /// select command runs.
    #[test]
    fn a_hellraiser_autoplay_without_a_choice_completes_inside_the_listener() {
        let fixture = hellraiser_kill(CardId::StrikeIronclad, true, 0, 0);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        take_branch_hits();
        let done = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert_eq!(take_branch_hits(), BranchHits::default());
        assert!(done.pending.is_none());
        assert!(done.frames.is_empty());
        assert!(done.piles.get(PileId::Play).is_empty());
        // The AutoPlayed Strike finished inside the listener, before the
        // enclosing Strike.
        assert_eq!(uids(&done, PileId::Discard), vec![2, 1]);
        assert_eq!(done.history.card_plays_finished_combat, 2);
    }

    /// #3387 non-choice witness: with Stratagem live but a non-empty Draw
    /// pile the Horn's Draw runs on the frame, completes inside the
    /// listener, and nothing is queued; the result is the synchronous
    /// command's (one card drawn, one energy gained, no frame).
    #[test]
    fn a_horn_draw_that_begins_no_choice_completes_inside_the_listener() {
        let mut fixture = stratagem_kill();
        let defend = fixture.atoms[1];
        push(&mut fixture, PileId::Draw, [card(9, defend)]);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        take_branch_hits();
        let done = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert_eq!(take_branch_hits(), BranchHits::default());
        assert!(done.pending.is_none());
        assert!(done.frames.is_empty());
        assert!(!done.fanouts.deferred_hook_action_is_queued());
        assert_eq!(done.piles.get(PileId::Hand).as_slice()[0].uid, 9);
        assert_eq!(done.cards_drawn_combat, 1);
        assert_eq!(done.energy, state.energy);

        // The same fight with Stratagem absent keeps the synchronous
        // command, with the same piles and energy.
        let mut plain = fixture.state.clone();
        plain.powers.set(PowerId::Stratagem, SlotWire::Int, 0);
        let (plain, plain_catalog) = cold(&plain, &fixture.catalog);
        let reference = apply_action(&plain, &plain_catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert_eq!(reference.piles, done.piles);
        assert_eq!(reference.energy, done.energy);
        assert_eq!(reference.monsters, done.monsters);
    }

    /// #3387 scope witness: a Doom kill at the enemy side end is a death in
    /// EndTurn, which is not a queued action; a choice its Horn Draw begins
    /// refuses by name.
    #[test]
    fn a_horn_choice_during_end_turn_refuses_by_name() {
        let mut fixture = stratagem_kill();
        fixture.state.piles.get_mut(PileId::Hand).make_mut().clear();
        fixture.state.monsters_mut()[0]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 5);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert_eq!(
            apply_action(&state, &catalog, &Action::EndTurn).map(|_| ()),
            Err(EngineRefusal::PowerOrderNotModeled(OUTSIDE_PLAYER_ACTION))
        );
    }

    /// [`stratagem_kill`] (with `extra` relics) with its Horn Draw issued
    /// directly in a player action's scope: the choice is queued and not yet
    /// published.
    fn queued_stratagem(extra: &[RelicId]) -> (HotState, Catalog) {
        let base = stratagem_kill();
        let mut fixture = fixture(
            &[CardId::StrikeIronclad, CardId::DefendIronclad],
            extra,
            &[5, 1_000],
        );
        assert_eq!(fixture.atoms, base.atoms);
        fixture.state = base.state;
        let (mut state, catalog) = cold(&fixture.state, &fixture.catalog);
        let _scope = ScopeGuard::enter(ActionScope::PlayerAction);
        take_branch_hits();
        super::super::draw::gremlin_horn_draw(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(take_branch_hits().queued, 1);
        assert!(state.fanouts.deferred_hook_action_is_queued());
        assert!(state.pending.is_none());
        assert!(state.frames.is_empty());
        (state, catalog)
    }

    /// Empty the Draw pile back into the Discard pile, so the next Draw
    /// reshuffles against Stratagem again.
    fn restock_discard(state: &mut HotState) {
        let cards = std::mem::take(state.piles.get_mut(PileId::Draw).make_mut());
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(cards);
    }

    /// #3387 branch witnesses for [`deferred_listener_draw`]'s refusals.
    #[test]
    fn deferred_listener_draw_refuses_each_unmodeled_context_by_name() {
        let fixture = stratagem_kill();
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let draw = |scope: Option<ActionScope>, state: &HotState, catalog: &Catalog| {
            let mut state = state.clone();
            let _scope = scope.map(ScopeGuard::enter);
            super::super::draw::gremlin_horn_draw(&mut state, catalog, &mut Vec::new())
        };
        assert_eq!(
            draw(None, &state, &catalog),
            Err(EngineRefusal::PowerOrderNotModeled(WITHOUT_RECEIPT))
        );
        assert_eq!(
            draw(Some(ActionScope::OtherAction), &state, &catalog),
            Err(EngineRefusal::PowerOrderNotModeled(OUTSIDE_PLAYER_ACTION))
        );
        assert_eq!(
            super::super::puzzle::with_tape_for_test(Vec::new(), || {
                draw(Some(ActionScope::PlayerAction), &state, &catalog)
            }),
            Err(EngineRefusal::PowerOrderNotModeled(
                INSIDE_RECEIPT_REEXECUTION
            ))
        );
        // A caller other than the Horn is not a deferred listener Draw.
        assert_eq!(
            deferred_listener_draw(
                &mut state.clone(),
                &catalog,
                1,
                DrawCaller::JossPaper,
                &mut Vec::new()
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        // A second reshuffle choice while the first is queued.
        let (mut queued, catalog) = queued_stratagem(&[RelicId::RelicCentennialPuzzle]);
        restock_discard(&mut queued);
        assert_eq!(
            draw(Some(ActionScope::PlayerAction), &queued, &catalog),
            Err(EngineRefusal::PowerOrderNotModeled(SECOND_HOOK_ACTION))
        );
        // A receipt-owned park (Centennial Puzzle's Draw reshuffling against
        // Stratagem) while the hook action is queued.
        let mut puzzle = queued.clone();
        puzzle.fanouts.set_puzzle_armed(true);
        assert_eq!(
            super::super::draw::centennial_puzzle_draw_one(&mut puzzle, &catalog, &mut Vec::new()),
            Err(EngineRefusal::PowerOrderNotModeled(BESIDE_A_CHOICE))
        );
    }

    /// #3387: only Stratagem's Draw selection and a Hellraiser-AutoPlayed
    /// Seeker or Sculpting selection are audited deferred choices, and only a
    /// Hellraiser AutoPlay may sit in a queued segment.
    #[test]
    fn only_audited_choices_can_be_deferred() {
        let mut fixture = fixture(&[CardId::Armaments, CardId::DefendIronclad], &[], &[1_000]);
        let (armaments, defend) = (fixture.atoms[0], fixture.atoms[1]);
        push(&mut fixture, PileId::Hand, [card(1, armaments)]);
        push(
            &mut fixture,
            PileId::Hand,
            (2..4).map(|uid| card(uid, defend)),
        );
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert!(!deferred_choice_is_audited(&state, &catalog));
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(
            parked
                .pending_card_play(parked.pending.as_deref().unwrap())
                .is_some()
        );
        assert!(!deferred_choice_is_audited(&parked, &catalog));
        // A manual card play (Armaments' own selection) is not a segment
        // child the Draw grammar nests.
        assert!(!segment_card_plays_are_hellraiser(&parked, 0));

        let fixture = stratagem_kill();
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        assert!(deferred_choice_is_audited(&parked, &catalog));
        assert!(segment_card_plays_are_hellraiser(&parked, 0));

        let fixture = hellraiser_kill(CardId::SeekerStrike, true, 0, 0);
        let (parked, _) = park_hellraiser_choice(&fixture);
        assert!(segment_card_plays_are_hellraiser(&parked, 0));
    }

    /// #3387 branch witnesses for [`publish_queued`].
    #[test]
    fn publish_queued_refuses_each_unmodeled_boundary_by_name() {
        let (queued, catalog) = queued_stratagem(&[]);
        let publish = |edit: &dyn Fn(&mut HotState)| {
            let mut state = queued.clone();
            edit(&mut state);
            publish_queued(&mut state).map(|published| (published, state))
        };
        let refusal = |edit: &dyn Fn(&mut HotState)| publish(edit).map(|(published, _)| published);

        // Nothing queued: nothing to publish.
        let mut empty = queued.clone();
        empty.fanouts.take_deferred_hook_action_for_test();
        assert_eq!(publish_queued(&mut empty), Ok(false));

        assert_eq!(
            refusal(&|state| {
                state.pending = Some(std::sync::Arc::new(
                    crate::hot::PendingSelection::relic_selection(),
                ));
            }),
            Err(EngineRefusal::PowerOrderNotModeled(BESIDE_A_CHOICE))
        );
        assert_eq!(
            refusal(&|state| state
                .frames
                .push(crate::frame::Frame::ConsumingShadowSideEnd { auth_uid: 0 })),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(
            refusal(&|state| state.history.over = true),
            Err(EngineRefusal::PowerOrderNotModeled(AT_COMBAT_END))
        );
        assert_eq!(
            refusal(&|state| state.fanouts.set_void_form_end_turn_requested(true)),
            Err(EngineRefusal::PowerOrderNotModeled(
                BESIDE_VOID_FORM_END_TURN
            ))
        );
        assert_eq!(
            refusal(&|state| {
                let card = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
                state.piles.get_mut(PileId::Hand).make_mut().push(card);
            }),
            Err(EngineRefusal::PowerOrderNotModeled(CHOICE_SOURCE_MOVED))
        );
        assert_eq!(
            refusal(&|state| {
                let card = state.piles.get_mut(PileId::Draw).make_mut().remove(0);
                state.piles.get_mut(PileId::Discard).make_mut().push(card);
            }),
            Err(EngineRefusal::PowerOrderNotModeled(CHOICE_SOURCE_MOVED))
        );
        assert_eq!(
            refusal(&|state| {
                let card = state.piles.get(PileId::Hand).as_slice()[0];
                state.card_states.set(
                    card.uid,
                    crate::hot::CardInstanceState {
                        damage_growth: 3,
                        ..Default::default()
                    },
                );
            }),
            Err(EngineRefusal::PowerOrderNotModeled(CHOICE_SOURCE_MOVED))
        );
        // A card left in (or added to) Play beside the queued segment.
        assert_eq!(
            refusal(&|state| {
                let card = state.piles.get(PileId::Hand).as_slice()[0];
                state
                    .piles
                    .get_mut(PileId::Play)
                    .make_mut()
                    .push(HotCard { uid: 41, ..card });
            }),
            Err(EngineRefusal::PowerOrderNotModeled(PLAY_PILE_MOVED))
        );
        // A Discard-only change (as the enclosing card's own finish makes)
        // publishes.
        let (published, state) = publish(&|state| {
            let card = state.piles.get(PileId::Hand).as_slice()[0];
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(HotCard { uid: 40, ..card });
        })
        .unwrap();
        assert!(published);
        assert!(!state.fanouts.deferred_hook_action_is_queued());
        assert!(
            state
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&state.frames)
                .is_some()
        );
        assert!(matches!(
            state.frames.as_slice(),
            [crate::frame::Frame::Draw { .. }]
        ));

        // A Select transaction publishes on top of its existing root: the
        // segment's records are rebased past the root's words.
        let mut rooted = queued.clone();
        let predecessor = HotBoundary::try_to_canonical(&fixture_root(), &catalog).unwrap();
        rooted
            .install_action_replay_root(&crate::hot::ActionReplayRecord {
                predecessor_json: predecessor.canonical_json().into_bytes(),
                action: crate::hot::ActionReplayRootAction::Play {
                    uid: 1,
                    target: Some(0),
                    selection_uid: None,
                },
                answers: Vec::new(),
            })
            .unwrap();
        let root_words = rooted.frames.word_len();
        assert!(root_words > 0);
        assert_eq!(publish_queued(&mut rooted), Ok(true));
        let [
            crate::frame::Frame::ActionReplay { .. },
            crate::frame::Frame::Draw { record },
        ] = rooted.frames.as_slice()
        else {
            panic!("{:#?}", rooted.frames);
        };
        assert_eq!(rooted.pending.as_deref().unwrap().frame_record, *record);
        assert_eq!(
            rooted.frames.draw(*record).unwrap().caller,
            DrawCaller::GremlinHorn
        );
        assert!(
            rooted
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&rooted.frames)
                .is_some()
        );
    }

    fn fixture_root() -> HotState {
        stratagem_kill().state
    }

    /// #3387: every arm of [`scope_for`].
    #[test]
    fn scope_for_names_the_native_action_each_transaction_runs() {
        use crate::hot::ActionReplayRootAction;
        let play = ActionReplayRootAction::Play {
            uid: 1,
            target: None,
            selection_uid: None,
        };
        let potion = ActionReplayRootAction::UsePotion {
            slot: 0,
            target: None,
        };
        let select = Action::Select {
            answer: crate::engine::SelectionAnswer::OptionIndex(0),
        };
        assert_eq!(
            scope_for(&STRIKE_THE_WEAK_TOADPOLE, None),
            ActionScope::PlayerAction
        );
        assert_eq!(
            scope_for(
                &Action::UsePotion {
                    slot: 0,
                    target: None
                },
                None
            ),
            ActionScope::PlayerAction
        );
        assert_eq!(scope_for(&select, Some(play)), ActionScope::PlayerAction);
        assert_eq!(scope_for(&select, Some(potion)), ActionScope::PlayerAction);
        assert_eq!(
            scope_for(&select, Some(ActionReplayRootAction::EndTurn)),
            ActionScope::OtherAction
        );
        assert_eq!(scope_for(&select, None), ActionScope::OtherAction);
        assert_eq!(scope_for(&Action::EndTurn, None), ActionScope::OtherAction);
    }

    /// #3387 AfterDeath witness inside a potion's action: Explosive Ampoule
    /// kills the 5-HP Toadpole and spares the other; the Horn's reshuffle
    /// choice is published on the UsePotion root after the potion finishes.
    #[test]
    fn a_horn_choice_inside_a_potion_waits_for_the_potion_to_finish() {
        let mut fixture = stratagem_kill();
        fixture.state.piles.get_mut(PileId::Hand).make_mut().clear();
        assert!(fixture.state.fanouts.set_potion_belt(
            vec![Some(crate::ids::PotionId::ExplosiveAmpoule)],
            false,
            false,
            false,
            false,
            true,
        ));
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let potion = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let parked = apply_action(&state, &catalog, &potion).unwrap().state;
        published_horn_draw(&parked);
        let crate::frame::Frame::ActionReplay { record } = parked.frames.as_slice()[0] else {
            unreachable!()
        };
        assert!(matches!(
            parked.frames.action_replay(record).unwrap().action,
            crate::hot::ActionReplayRootAction::UsePotion { .. }
        ));
        assert!(parked.fanouts.potion_slots().iter().all(Option::is_none));
        assert!(parked.monsters[0].hp <= 0);
        assert!(parked.monsters[1].hp < 1_000);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 3);
        }
    }

    /// #3387: the boundary refuses a state still carrying a queued hook
    /// action, in both directions.
    #[test]
    fn a_queued_hook_action_never_crosses_the_boundary() {
        let (queued, catalog) = queued_stratagem(&[]);
        assert!(HotBoundary::try_to_canonical(&queued, &catalog).is_err());
        assert!(!crate::boundary::batch_nine_relic_state_is_exact(
            &queued, &catalog
        ));
    }

    /// #3387: a detached segment re-attaches onto a different base with
    /// every record index rebased, and a forged caller does not load.
    #[test]
    fn a_published_hook_action_rebases_onto_the_root_and_its_caller_is_pinned() {
        let fixture = stratagem_kill();
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = apply_action(&state, &catalog, &STRIKE_THE_WEAK_TOADPOLE)
            .unwrap()
            .state;
        let crate::frame::Frame::ActionReplay { record: root } = parked.frames.as_slice()[0] else {
            unreachable!()
        };
        let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[1] else {
            unreachable!()
        };
        assert!(parked.frames.action_replay(root).is_some());
        assert!(parked.frames.draw(record).is_some());
        assert_eq!(parked.pending.as_deref().unwrap().frame_record, record);

        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let text = wire.canonical_json();
        assert!(text.contains("\"gremlin_horn\""));
        for forged in ["\"joss_paper\"", "\"turn_start\"", "\"potion_epilogue\""] {
            let document: crate::canonical::CanonicalStateV2 =
                serde_json::from_str(&text.replace("\"gremlin_horn\"", forged)).unwrap();
            let loaded = HotBoundary::catalog_from_canonical(&document).and_then(|catalog| {
                HotBoundary::from_canonical(&document, &catalog).map(|state| (state, catalog))
            });
            let refused = match loaded {
                Err(_) => true,
                Ok((state, catalog)) => admit(&document, &state, &catalog).is_err(),
            };
            assert!(refused, "{forged}");
        }
    }
}
