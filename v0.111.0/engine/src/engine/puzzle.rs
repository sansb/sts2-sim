//! Receipt-owned Draws (#3114, #3115, #3201): Centennial Puzzle's parked
//! Draw, Swift's parked OnPlay Draw and Joss Paper's parked threshold Draw,
//! carried by the action's ActionReplay receipt.
//!
//! # Native
//!
//! `CentennialPuzzle/<AfterDamageReceived>d__10::MoveNext` RVA `0x321758`
//! (v0.111.0, SHA-256 `9cb4f1ad…`) returns at IL_005c-0064 once
//! `UsedThisCombat` is set, sets it at IL_006f-0071, and awaits three one-card
//! `CardPileCmd.Draw(choiceContext, Owner)` calls (IL_007f-0116). The relic is
//! a listener in `Hook/<AfterDamageReceived>d__26::MoveNext` RVA `0x3cd698`,
//! which awaits every ordinary listener serially (IL_0045-0119) before the
//! Late walk (IL_013f-0241). That hook is awaited by
//! `CreatureCmd/<Damage>d__12::MoveNext` RVA `0x3e96c8` at IL_0d86, before the
//! command's killed-creature batch (IL_0ead-0eb4). Every player-damage caller
//! awaits that command: an enemy `AttackCommand/<Execute>d__90::MoveNext` RVA
//! `0x3f19c0` awaits it per hit at IL_0743 and only then re-tests the dealer
//! (IL_015c) and reaches `AfterAttack` (IL_0845); Thorns, power ticks and card
//! self-damage await it from their own bodies.
//!
//! `Enchantments.Swift/<OnPlay>d__4::MoveNext` RVA `0x3885c0` returns when
//! `Status != 0`, sets `Status = 1`, and then awaits one
//! `CardPileCmd.Draw(choiceContext, Amount, Card.Owner, false)`
//! (IL_001d-0053). The enchantment's OnPlay is itself awaited by the card
//! play after its body, before `CardPlayFinished`.
//!
//! `JossPaper/<DrawIfThresholdMet>d__27::MoveNext` RVA `0x327888` returns
//! while `CardsExhausted < ExhaustAmount` (IL_0020-0047; 5 by
//! `get_CanonicalVars` RVA `0x9540b` IL_0009-000f), fires its visuals
//! unawaited (`TaskHelper.RunSafely`, IL_004c-0057), awaits one
//! `CardPileCmd.Draw(choiceContext, CardsExhausted / ExhaustAmount, Owner,
//! false)` (IL_0058-00eb), and only then writes
//! `CardsExhausted %= ExhaustAmount` from the live counter (IL_00ec-0109).
//! Its two callers await it as their tail:
//! `JossPaper/<AfterCardExhausted>d__25::MoveNext` RVA `0x3275a0`
//! (owner test IL_001d-0030, `causedByEthereal` tally IL_0035-004d, else
//! `CardsExhausted += 1` then the await, IL_004f-00bd) and
//! `JossPaper/<AfterSideTurnEnd>d__26::MoveNext` RVA `0x3276ac`
//! (`CardsExhausted += EtherealCount; EtherealCount = 0` at IL_003a-004f,
//! then the await at IL_0054-00b2). Each is a listener its hook awaits
//! serially, so the rest of the command is again a pure continuation.
//!
//! So when either Draw suspends on a player choice (a selecting Hellraiser
//! Strike AutoPlay, or Stratagem's `AfterShuffle` selection), the whole
//! remainder of the enclosing command is a pure continuation: nothing runs
//! between the park and the answer, and after the answer the native command
//! resumes exactly where its awaits left it.
//!
//! # Carrier
//!
//! That remainder is Rust stack, not frames: the rest of the hit loop, the
//! move program, the enemy phase, a card play's finish, a turn transition.
//! Rather than persisting a frame for every such cursor, the carrier owns it
//! through the public action's existing ActionReplay receipt, which already
//! stores the exact depth-zero predecessor document, the root action and
//! every answer.
//!
//! * **Park.** With no answer left, [`receipt_owned_draw`] snapshots the
//!   state it is drawing into and unwinds the action with the private
//!   [`PARK_SITE`] refusal. The public transaction ([`CarrierGuard`])
//!   publishes that snapshot as the parked state. The snapshot is taken where
//!   the Draw ran, not from the transaction's own successor, because the
//!   damage pipeline commits through clones (`*state = next`). The last park
//!   of the transaction wins: a rehearsal probe's park either propagates
//!   (so no committed run follows it, and the probe, a clone of the same
//!   state running the same code, holds the committed prefix) or is
//!   swallowed, in which case the committed run parks after it.
//! * **Resume.** A selection answer on a parked state re-executes the root
//!   action from the receipt's predecessor with the receipt's answers plus
//!   the new one as a tape ([`resume_receipt_owned_park`]). Answers are read
//!   in order through the fanouts' `replay_tape_cursor`, a per-lineage cursor:
//!   public Select applications and in-place answers both advance the state
//!   that consumed them, and a rehearsal probe advances only its own clone,
//!   so a probe cannot consume the committed run's answers.
//! * **Inline answers.** At a receipt-owned Draw with a tape answer, the
//!   answer is resolved in place and the frames above the Draw are driven
//!   back to its depth ([`inline_depth`]), after which the Rust caller
//!   continues.
//!
//! The parked state's frames above the Draw are ordinary Draw/CardPlay
//! children, validated by the same grammar as a turn-start Draw; frames below
//! it belong to the unwound caller and are authenticated, like everything
//! else in a parked document, by the receipt's replay on import.

use std::cell::{Cell, RefCell};

use crate::catalog::Catalog;
use crate::hot::{
    ActionReplayAnswer, ActionReplayRecord, ActionReplayRootAction, DrawCaller, HotState,
};

use super::{Action, EngineRefusal, Event, SelectionAnswer, SelectionRef};

/// The private unwinding signal of a parked receipt-owned Draw. It is never
/// published: [`CarrierGuard::settle`] converts it or refuses.
pub(crate) const PARK_SITE: &str = "receipt-owned Draw parked";

thread_local! {
    /// Whether a public transaction is open to receive a park snapshot.
    static CARRIER_OPEN: Cell<bool> = const { Cell::new(false) };
    /// The last parked snapshot of the open transaction.
    static SNAPSHOT: RefCell<Option<HotState>> = const { RefCell::new(None) };
    /// The answers a re-execution consumes.
    static TAPE: RefCell<Option<Vec<ActionReplayAnswer>>> = const { RefCell::new(None) };
    /// Depth of the receipt-owned Draw frame whose children are resumed in
    /// place.
    static INLINE_DEPTH: Cell<Option<usize>> = const { Cell::new(None) };
    /// The innermost deferred-choice hook listener being run, as the name a
    /// park inside it refuses under, or `None` where a park's choice is
    /// proven to see the same state natively (#3386, see
    /// [`DeferredChoiceListener`]).
    static DEFERRED_CHOICE_LISTENER: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// A listener of a native deferred-choice hook walk is running (#3386).
///
/// # Native
///
/// Six `Hook` walks give each listener its own `HookPlayerChoiceContext`
/// and await `AssignTaskAndWaitForPauseOrCompletion`, not the listener:
/// `Hook/<AfterDeath>d__28::MoveNext` RVA `0x3cd984` (IL_008f, IL_00b6),
/// `<AfterDiedToDoom>d__30` `0x3cdb4c` (IL_007e, IL_0099),
/// `<BeforeSideTurnStart>d__74` `0x3d39a8` (IL_007e, IL_00a5),
/// `<BeforeSideTurnEnd>d__80` `0x3d34f4` (VeryEarly IL_0093-0124, Early
/// IL_01ab-0242, ordinary IL_02c9-0360), `<BeforeFlush>d__33` `0x3d2acc`
/// (IL_008f-011a, Late IL_01a1-0232) and `<AfterSideTurnEnd>d__81`
/// `0x3d11c0` (IL_0093-0124, Late IL_0219-02b0) (v0.111.0, SHA-256
/// `9cb4f1ad…`). That await
/// (`HookPlayerChoiceContext/<AssignTaskAndWaitForPauseOrCompletion>d__34`
/// RVA `0x3d62fc` IL_005c-0070) is a `WhenAny` of the listener's task and the
/// context's paused source. When the listener begins a player choice,
/// `<SignalPlayerChoiceBegun>d__37` RVA `0x3d64f4` moves the rest of it into
/// a queued `GenericHookGameAction` (`RequestEnqueueHookAction`, IL_0288-028e),
/// releases the paused source (IL_0294-0299) and waits for that action to
/// start (IL_029e-02a4) before the choice is shown (the local-player branch,
/// IL_0268-0279). So every LATER listener of the walk is started before the
/// choice resolves, and the paused listener's own remainder runs after
/// them. BeforeSideTurnEnd, BeforeFlush and
/// AfterSideTurnEnd then `WhenAll` the listeners' `WaitForCompletion` tasks
/// (`0x3d34f4` IL_03a8 once after all three passes; `0x3d2acc` IL_027a once
/// after both; `0x3d11c0` IL_016c after the ordinary pass and IL_02f8 after
/// Late), so nothing past the walk runs first. AfterDeath, AfterDiedToDoom
/// and BeforeSideTurnStart have no `WhenAll`: the enclosing command runs on
/// before the choice too.
///
/// A receipt-owned park (the module docs) assumes nothing runs between the
/// park and the answer. Inside such a listener that holds only when every
/// later listener commutes with the choice and its continuation. The Rust
/// site that models the walk enters this guard with the refusal name, or
/// with `None` where it proved commutation; the innermost guard wins,
/// because the choice pauses the innermost listener's own context.
///
/// # A choice that resolves without a prompt (#3485)
///
/// The deferral does not wait for a prompt. The combat `CardSelectCmd`
/// entry points signal the context before they read their options and
/// before they decide to auto-take them: `<FromCombatPile>d__20::MoveNext`
/// RVA `0x3e5e84` signals at IL_00a2-00ae, reads `pile.Cards` at
/// IL_010c-013b and auto-takes at IL_0140-017c (an empty list at
/// IL_0141-0155); `<FromHand>d__28` RVA `0x3e7568` signals at IL_00b3 and
/// reads at IL_010d-014c; `<FromHandForDiscard>d__29` RVA `0x3e7a64`
/// forwards to `FromHand` (IL_005e); `<FromHandForUpgrade>d__30` RVA
/// `0x3e7b6c` signals at IL_00af and reads at IL_0115; `<FromSimpleGrid>d__18`
/// RVA `0x3e8044` signals at IL_00b7 before its count tests
/// (IL_011c-0156). Only `IsEnding`/`IsOverOrEnding` or a set `Selector`
/// skips the signal. So an auto-resolved selection moves the rest of the
/// listener behind its later peers exactly as a prompted one does, and its
/// option list is read only then. Rust resolves it inline, so under a named
/// wall it refuses ([`AUTO_RESOLVED_IN_DEFERRED_LISTENER`], counted by
/// `hook_action::note_unprompted_select`). Under `None` the later peers
/// commute with the choice and its continuation, so the inline order is
/// exact and nothing is counted.
///
/// No listener of the six walks calls a `CardSelectCmd` entry point in its
/// own body. Each reaches one only through a Draw that its command issues
/// on the same context (only the six walks and `CombatManager`'s own turn
/// phases construct a `HookPlayerChoiceContext`), and then through
/// `StratagemPower/<AfterShuffle>d__4` RVA `0x34688c` (`FromCombatPile`,
/// IL_0046-0078) or through `HellraiserPower/<AfterCardDrawnEarly>d__7` RVA
/// `0x33c1a8`'s AutoPlay (IL_011e-012e) of Seeker Strike (`FromCombatPile`,
/// `<OnPlay>d__5` RVA `0x3b9754` IL_013d-0172) or Sculpting Strike
/// (`FromHand`, `<OnPlay>d__7` RVA `0x3b8cec` IL_00de-0115). Those are the
/// three Rust arms that call `note_unprompted_select`.
pub(crate) struct DeferredChoiceListener {
    outer: Option<&'static str>,
    wall: Option<&'static str>,
    signals: super::hook_action::UnsignaledListener,
}

impl DeferredChoiceListener {
    pub(crate) fn enter(wall: Option<&'static str>) -> Self {
        let outer = DEFERRED_CHOICE_LISTENER.with(|listener| listener.replace(wall));
        let signals = if wall.is_some() {
            super::hook_action::UnsignaledListener::enter()
        } else {
            super::hook_action::UnsignaledListener::exempt()
        };
        Self {
            outer,
            wall,
            signals,
        }
    }

    /// Leave the listener, refusing by name if a select inside it resolved
    /// without a prompt under a named wall (#3485). The refusal replaces any
    /// other result, including a park unwinding from an inner commuting
    /// listener: the inline auto-take has already run too early.
    pub(crate) fn settle<T>(self, result: Result<T, EngineRefusal>) -> Result<T, EngineRefusal> {
        if self.wall.is_some() && self.signals.unprompted_signals() > 0 {
            return Err(EngineRefusal::PowerOrderNotModeled(
                AUTO_RESOLVED_IN_DEFERRED_LISTENER,
            ));
        }
        result
    }
}

impl Drop for DeferredChoiceListener {
    fn drop(&mut self) {
        DEFERRED_CHOICE_LISTENER.with(|listener| listener.set(self.outer));
    }
}

/// A select inside a deferred-choice listener under a named wall resolved
/// without a prompt, and so ran before the later listeners that native
/// starts first (#3485, [`DeferredChoiceListener`]).
pub(crate) const AUTO_RESOLVED_IN_DEFERRED_LISTENER: &str =
    "deferred-choice listener choice resolved without a prompt";

/// Backstop names for a park inside each modeled deferred-choice walk whose
/// later listeners were not proven to commute (#3386).
pub(crate) const AFTER_SIDE_TURN_END_WALL: &str =
    "receipt-owned Draw inside a deferred AfterSideTurnEnd listener";
pub(crate) const BEFORE_SIDE_TURN_END_WALL: &str =
    "receipt-owned Draw inside a deferred BeforeSideTurnEnd listener";
pub(crate) const BEFORE_SIDE_TURN_START_WALL: &str =
    "receipt-owned Draw inside a deferred BeforeSideTurnStart listener";
pub(crate) const AFTER_DEATH_WALL: &str =
    "receipt-owned Draw inside a deferred AfterDeath listener";
/// The named sites the admission gate also refuses (#3386).
pub(crate) const JOSS_SIDE_END_WALL: &str =
    "Joss Paper side-end Draw before a later AfterSideTurnEnd listener";
pub(crate) const PAELS_EYE_JOSS_WALL: &str = "Joss Paper Draw inside Pael's Eye BeforeSideTurnEnd";
pub(crate) const CONSTRICT_PUZZLE_WALL: &str =
    "Centennial Puzzle Draw inside Constrict AfterSideTurnEnd";

/// Whether the admission gate refuses `state` naming `wall` (#3386 witnesses
/// outside this module; the hot engine modules may not name the gate).
#[cfg(test)]
pub(crate) fn root_refuses_under_for_test(
    state: &HotState,
    catalog: &Catalog,
    wall: &'static str,
) -> bool {
    let wire = crate::boundary::HotBoundary::try_to_canonical(state, catalog).unwrap();
    super::admission::admit(&wire, state, catalog).is_err_and(|refusal| {
        refusal.contains(super::admission::MissingCapability::ArgumentShape(wall))
    })
}

/// The name a park refuses under inside the current deferred-choice
/// listener, if any.
pub(crate) fn deferred_choice_wall() -> Option<&'static str> {
    DEFERRED_CHOICE_LISTENER.with(Cell::get)
}

/// Whether `caller`'s suffix is owned by the action's receipt rather than by
/// a frame beneath its Draw.
pub(crate) const fn is_receipt_owned(caller: DrawCaller) -> bool {
    matches!(
        caller,
        DrawCaller::CentennialPuzzle | DrawCaller::SwiftEnchantment | DrawCaller::JossPaper
    )
}

/// Marks one public transaction as the publisher of a parked snapshot.
pub(crate) struct CarrierGuard {
    open: bool,
    snapshot: Option<HotState>,
}

impl CarrierGuard {
    pub(crate) fn enter() -> Self {
        Self {
            open: CARRIER_OPEN.with(|open| open.replace(true)),
            snapshot: SNAPSHOT.with(|snapshot| snapshot.borrow_mut().take()),
        }
    }

    /// Publish this transaction's parked Draw into `next`, or refuse.
    ///
    /// A park that some intermediate caller swallowed or rewrote is a
    /// refusal, never a published state.
    pub(crate) fn settle(
        &self,
        next: &mut HotState,
        result: Result<(), EngineRefusal>,
    ) -> Result<(), EngineRefusal> {
        let snapshot = SNAPSHOT.with(|snapshot| snapshot.borrow_mut().take());
        match (result, snapshot) {
            (Err(EngineRefusal::MalformedArgs(site)), Some(parked))
                if site == PARK_SITE
                    && parked.pending.is_some()
                    && parked_draw_depth(&parked).is_some() =>
            {
                *next = parked;
                Ok(())
            }
            (_, Some(_)) => Err(EngineRefusal::ContinuationNotModeled),
            (Err(EngineRefusal::MalformedArgs(site)), None) if site == PARK_SITE => {
                Err(EngineRefusal::ContinuationNotModeled)
            }
            (result, None) => result,
        }
    }
}

impl Drop for CarrierGuard {
    fn drop(&mut self) {
        CARRIER_OPEN.with(|open| open.set(self.open));
        SNAPSHOT.with(|snapshot| *snapshot.borrow_mut() = self.snapshot.take());
    }
}

fn park(state: &HotState) -> Result<(), EngineRefusal> {
    // #3386: native runs later deferred-choice listeners before this choice.
    if let Some(wall) = deferred_choice_wall() {
        return Err(EngineRefusal::PowerOrderNotModeled(wall));
    }
    // #3387: a queued hook action would have to cross this park's boundary.
    if state.fanouts.deferred_hook_action_is_queued() {
        return Err(EngineRefusal::PowerOrderNotModeled(
            super::hook_action::BESIDE_A_CHOICE,
        ));
    }
    if !CARRIER_OPEN.with(Cell::get) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    // The LAST park wins. A park unwinds to its rehearsal site, which either
    // propagates it (no later park) or swallows it and runs the committed
    // command, which then parks at the same await on the committed state.
    SNAPSHOT.with(|snapshot| *snapshot.borrow_mut() = Some(state.clone()));
    Err(EngineRefusal::MalformedArgs(PARK_SITE))
}

/// Test-only branch witnesses (#2700): which carrier branches a test reached.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BranchHits {
    /// Public Selects the re-execution driver applied at ordinary parks.
    pub driver_selects: u32,
    /// Tape answers consumed in place that were not the tape's last.
    pub inline_non_last: u32,
    /// In-place answers after which the same Draw parked again.
    pub inline_continue: u32,
    /// Tape answers consumed in place by a History Course dupe (#3309).
    pub history_course_answers: u32,
    /// Inline answers after which the dupe's frames were driven back to its
    /// History Course phase (#3309).
    pub history_course_drives: u32,
}

#[cfg(test)]
thread_local! {
    static BRANCH_HITS: Cell<BranchHits> = const {
        Cell::new(BranchHits {
            driver_selects: 0,
            inline_non_last: 0,
            inline_continue: 0,
            history_course_answers: 0,
            history_course_drives: 0,
        })
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

struct TapeGuard(Option<Vec<ActionReplayAnswer>>);

impl TapeGuard {
    fn install(answers: Vec<ActionReplayAnswer>) -> Self {
        Self(TAPE.with(|tape| tape.replace(Some(answers))))
    }
}

impl Drop for TapeGuard {
    fn drop(&mut self) {
        TAPE.with(|tape| *tape.borrow_mut() = self.0.take());
    }
}

/// Run `body` as if a receipt re-execution were consuming `answers` (#3387
/// branch witnesses outside this module).
#[cfg(test)]
pub(crate) fn with_tape_for_test<T>(
    answers: Vec<ActionReplayAnswer>,
    body: impl FnOnce() -> T,
) -> T {
    let _tape = TapeGuard::install(answers);
    body()
}

/// Whether a re-execution is running. Its intermediate public applications
/// leave the receipt's answer list to [`resume_receipt_owned_park`].
pub(crate) fn tape_is_installed() -> bool {
    TAPE.with(|tape| tape.borrow().is_some())
}

/// The answer at `state`'s lineage cursor, advancing that cursor, and whether
/// it was the tape's last.
fn take_answer(state: &mut HotState) -> Result<Option<(ActionReplayAnswer, bool)>, EngineRefusal> {
    TAPE.with(|tape| {
        let tape = tape.borrow();
        let Some(tape) = tape.as_ref() else {
            return Ok(None);
        };
        let cursor = state.fanouts.replay_tape_cursor();
        let Some(answer) = tape.get(usize::from(cursor)).copied() else {
            return Ok(None);
        };
        let cursor = cursor
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("replay tape cursor"))?;
        state.fanouts.set_replay_tape_cursor(cursor);
        Ok(Some((answer, usize::from(cursor) == tape.len())))
    })
}

struct InlineGuard(Option<usize>);

impl InlineGuard {
    fn enter(depth: usize) -> Self {
        Self(INLINE_DEPTH.with(|inline| inline.replace(Some(depth))))
    }
}

impl Drop for InlineGuard {
    fn drop(&mut self) {
        INLINE_DEPTH.with(|inline| inline.set(self.0));
    }
}

/// The receipt-owned Draw depth whose children are being resumed in place:
/// the transient runtime-only proof that lets such a stack run without its
/// ActionReplay root, which the enclosing transaction installs on publish.
pub(crate) fn inline_depth() -> Option<usize> {
    INLINE_DEPTH.with(Cell::get)
}

/// Position of the innermost receipt-owned frame, if the stack has one: a
/// receipt-owned Draw, or History Course's AutoPre phase (#3309), whose
/// awaited dupe AutoPlay is owned the same way.
///
/// Only the innermost one can be parked on: an outer one is being resumed
/// in place by the Rust frames of the same transaction.
pub(crate) fn parked_draw_depth(state: &HotState) -> Option<usize> {
    state
        .frames
        .as_slice()
        .iter()
        .enumerate()
        .rev()
        .find_map(|(position, frame)| match *frame {
            crate::frame::Frame::Draw { record }
                if state
                    .frames
                    .draw(record)
                    .is_some_and(|draw| is_receipt_owned(draw.caller)) =>
            {
                Some(position)
            }
            crate::frame::Frame::Phase { record }
                if state.frames.auto_pre_history_course_phase(record).is_some() =>
            {
                Some(position)
            }
            _ => None,
        })
}

/// History Course's awaited AutoPlay of its dupe, owned by the action's
/// receipt (#3309).
///
/// `HistoryCourse/<AfterAutoPrePlayPhaseEntered>d__2::MoveNext` RVA
/// `0x326aa8` (v0.111.0, SHA-256 `9cb4f1ad…`) awaits one
/// `CardCmd.AutoPlay(choiceContext, CreateDupe(card), null, …)`
/// (IL_0086-00a1), resumes at IL_00d0-00ee, and returns (IL_00f3 `leave` to
/// IL_010e `SetResult`): nothing of its own runs after the await. The
/// AutoPlay itself, `CardCmd/<AutoPlay>d__0::MoveNext` RVA `0x3df9d4`,
/// ends by awaiting `CardModel.OnPlayWrapper` (IL_062a, RVA `0x31b8d0`)
/// and leaves straight to `SetResult` (IL_067f-06dc), so the dupe's
/// CardPlay is its last await too. The
/// listener is awaited by the AutoPre/Normal walk, so everything after the
/// dupe's CardPlay — later AutoPre listeners, the phase terminal and the
/// rest of the turn transition — is a pure continuation, exactly the Rust
/// suffix the receipt re-executes (see the module docs).
///
/// The dupe is played on a resumable CardPlay frame above an AutoPre
/// History Course phase record naming the dupe's uid. A selection inside
/// that play (the dupe's own selector, or a selecting Hellraiser Strike its
/// Draw reaches) either takes its tape answer in place, after which the
/// frames above the phase are driven back to it, or parks the action.
pub(crate) fn receipt_owned_history_course_autoplay(
    state: &mut HotState,
    catalog: &Catalog,
    source: crate::hot::FrozenAutoBatchEntry,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let depth = state.frames.len();
    let dupe_uid = state.next_card_uid;
    state
        .frames
        .push_auto_pre_history_course_phase(dupe_uid)
        .ok_or(EngineRefusal::CounterOverflow(
            "AutoPre History Course phase",
        ))?;
    super::play::autoplay_history_course_dupe(state, catalog, source, events)?;
    loop {
        if state.pending.is_none() {
            if state.frames.len() != depth + 1
                || state.frames.pop_top_auto_pre_history_course_phase() != Some(dupe_uid)
            {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            return Ok(());
        }
        if parked_draw_depth(state) != Some(depth) || state.frames.len() <= depth + 1 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let Some((answer, last)) = take_answer(state)? else {
            return park(state);
        };
        if last {
            events.clear();
        }
        #[cfg(test)]
        {
            hit(|hits| hits.history_course_answers += 1);
            if !last {
                hit(|hits| hits.inline_non_last += 1);
            }
        }
        let _inline = InlineGuard::enter(depth);
        resume_inline_answer(state, catalog, answer, events)?;
        if state.pending.is_none() && state.frames.len() > depth + 1 {
            #[cfg(test)]
            hit(|hits| hits.history_course_drives += 1);
            super::play::drive_receipt_owned_history_course_inline(state, catalog, events, depth)?;
        }
        #[cfg(test)]
        if state.pending.is_some() {
            hit(|hits| hits.inline_continue += 1);
        }
    }
}

/// Issue one receipt-owned `CardPileCmd.Draw` of `n` cards (#3114, #3115).
///
/// The Draw runs on the resumable frame with `caller`. With a tape answer
/// the suspended choice is resolved in place and the Draw's children are
/// driven until the Draw frame itself has returned; the Rust caller then
/// continues. Without one, the whole action parks (see the module docs).
pub(crate) fn receipt_owned_draw(
    state: &mut HotState,
    catalog: &Catalog,
    n: usize,
    caller: DrawCaller,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !is_receipt_owned(caller) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let depth = state.frames.len();
    match super::draw::draw_cards_for_potion(state, catalog, n, caller, events)? {
        super::draw::PotionDrawResult::Complete => return Ok(()),
        super::draw::PotionDrawResult::Suspended => {}
    }
    loop {
        if state.pending.is_none() || parked_draw_depth(state) != Some(depth) {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let Some((answer, last)) = take_answer(state)? else {
            return park(state);
        };
        if last {
            // Events before the final answer were published by the earlier
            // transitions this re-execution retraces.
            events.clear();
        }
        #[cfg(test)]
        if !last {
            hit(|hits| hits.inline_non_last += 1);
        }
        let _inline = InlineGuard::enter(depth);
        resume_inline_answer(state, catalog, answer, events)?;
        if state.pending.is_none() && state.frames.len() > depth {
            super::play::drive_receipt_owned_draw_inline(state, catalog, events, depth)?;
        }
        match (state.pending.is_some(), state.frames.len()) {
            (false, len) if len == depth => return Ok(()),
            (true, len) if len > depth => {
                #[cfg(test)]
                hit(|hits| hits.inline_continue += 1);
                continue;
            }
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        }
    }
}

fn resume_inline_answer(
    state: &mut HotState,
    catalog: &Catalog,
    answer: ActionReplayAnswer,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let pending = state
        .pending
        .as_deref()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if pending.stratagem_draw_record(&state.frames).is_some() {
        let ActionReplayAnswer::OptionIndex(ordinal) = answer else {
            return Err(EngineRefusal::ContinuationNotModeled);
        };
        return super::draw::resume_stratagem_selection(state, catalog, ordinal, events);
    }
    if state.pending_card_play(pending).is_some() {
        let answer = match answer {
            ActionReplayAnswer::CardUid(uid) => SelectionAnswer::CardUid(uid),
            ActionReplayAnswer::OptionIndex(index) => SelectionAnswer::OptionIndex(index),
        };
        return super::play::resume_selection(state, catalog, answer, events);
    }
    Err(EngineRefusal::ContinuationNotModeled)
}

fn root_action(action: ActionReplayRootAction) -> Action {
    match action {
        ActionReplayRootAction::Play {
            uid,
            target,
            selection_uid,
        } => Action::Play {
            uid,
            target,
            selection: SelectionRef::new(selection_uid),
        },
        ActionReplayRootAction::EndTurn => Action::EndTurn,
        ActionReplayRootAction::UsePotion { slot, target } => Action::UsePotion { slot, target },
    }
}

fn select(answer: ActionReplayAnswer) -> Action {
    Action::Select {
        answer: match answer {
            ActionReplayAnswer::CardUid(uid) => SelectionAnswer::CardUid(uid),
            ActionReplayAnswer::OptionIndex(index) => SelectionAnswer::OptionIndex(index),
        },
    }
}

/// Answer a parked receipt-owned Draw by re-executing its receipt.
///
/// The predecessor is rebuilt from the receipt's own canonical bytes against
/// the running catalog, the root action is re-applied, and the answers are
/// consumed in order at the awaits they answered. The result is published
/// with the receipt rewritten to the complete answer list, or with no
/// receipt once the action has finished.
#[cold]
#[inline(never)]
pub(crate) fn resume_receipt_owned_park(
    catalog: &Catalog,
    replay: &ActionReplayRecord,
    answer: ActionReplayAnswer,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    let document: crate::canonical::CanonicalStateV2 =
        serde_json::from_slice(&replay.predecessor_json)
            .map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    let predecessor = crate::boundary::HotBoundary::from_canonical(&document, catalog)
        .map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    if predecessor.fanouts.replay_tape_cursor() != 0 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let mut answers = replay.answers.clone();
    answers.push(answer);
    let mut scratch = Vec::new();
    let mut resumed = {
        let _tape = TapeGuard::install(answers.clone());
        let mut resumed = super::apply_action_with_replay_witness(
            &predecessor,
            catalog,
            &root_action(replay.action),
            &mut scratch,
        )?;
        while let Some(&answer) = answers.get(usize::from(resumed.fanouts.replay_tape_cursor())) {
            // An answer left over after a completed action, or one the
            // original transcript gave at a non-owned park, is a public Select.
            if resumed.pending.is_none() || parked_draw_depth(&resumed).is_some() {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            let mut input = resumed;
            let cursor = input
                .fanouts
                .replay_tape_cursor()
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("replay tape cursor"))?;
            input.fanouts.set_replay_tape_cursor(cursor);
            #[cfg(test)]
            hit(|hits| hits.driver_selects += 1);
            resumed = super::apply_action_with_replay_witness(
                &input,
                catalog,
                &select(answer),
                &mut scratch,
            )?;
        }
        if usize::from(resumed.fanouts.replay_tape_cursor()) != answers.len() {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        resumed
    };
    resumed.fanouts.set_replay_tape_cursor(0);
    // A finished action may still stop at the next turn's independent relic
    // choice, which the dispatcher publishes without a receipt.
    if let Some(crate::frame::Frame::ActionReplay { record }) =
        resumed.frames.as_slice().first().copied()
    {
        let installed = resumed
            .frames
            .action_replay(record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        if resumed.pending.is_none()
            || installed.predecessor_json != replay.predecessor_json
            || installed.action != replay.action
            || !installed.answers.is_empty()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        for answer in answers {
            let Some(crate::frame::Frame::ActionReplay { record }) =
                resumed.frames.as_slice().first().copied()
            else {
                return Err(EngineRefusal::ContinuationNotModeled);
            };
            resumed
                .append_action_replay_answer(record, answer)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
        }
    } else if !resumed.frames.is_empty()
        || resumed
            .pending
            .as_deref()
            .is_some_and(|pending| !pending.is_relic_selection())
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if !resumed
        .frames
        .continuation_store_is_valid(resumed.pending.as_deref())
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    events.clear();
    events.extend(scratch);
    Ok(resumed)
}

#[cfg(test)]
mod tests {
    use super::{CarrierGuard, PARK_SITE, parked_draw_depth};
    use crate::boundary::HotBoundary;
    use crate::catalog::{CardAtom, CardIdentity, Catalog, CatalogBuilder};
    use crate::engine::admission::{MissingCapability, PHASE_ORDINARY_ACTIONS, admit};
    use crate::engine::{Action, EngineRefusal, SelectionRef, apply_action, legal_actions};
    use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster, HotState, PileId};
    use crate::ids::{CardId, MonsterKind, PowerId, RelicId};
    use crate::powers::SlotWire;

    fn plain(id: CardId) -> CardIdentity {
        CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        }
    }

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
    /// Returns every terminal and the number of parked states visited.
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

    /// A Toadpole fight holding Centennial Puzzle, with `cards` interned in
    /// order (their atoms are returned in the same order).
    fn fixture(cards: &[CardId], loop_pos: i32) -> Fixture {
        let identities = cards.iter().map(|id| plain(*id)).collect::<Vec<_>>();
        fixture_with(&identities, &[RelicId::RelicCentennialPuzzle], loop_pos)
    }

    fn fixture_with(cards: &[CardIdentity], relics: &[RelicId], loop_pos: i32) -> Fixture {
        fixture_in_order(cards, relics, loop_pos, false)
    }

    /// [`fixture_with`], with the inventory vouched for as dispatch order or
    /// not (#3400).
    fn fixture_in_order(
        cards: &[CardIdentity],
        relics: &[RelicId],
        loop_pos: i32,
        vouched: bool,
    ) -> Fixture {
        let mut builder = CatalogBuilder::new();
        let atoms = cards
            .iter()
            .map(|identity| builder.intern_reachable(*identity).unwrap())
            .collect();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.set_relics_ordered(relics, vouched).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 20;
        state.exact_piles = true;
        state
            .fanouts
            .set_puzzle_armed(relics.contains(&RelicId::RelicCentennialPuzzle));
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
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = loop_pos;
        state.monsters = std::sync::Arc::new(vec![monster]);
        Fixture {
            state,
            catalog,
            atoms,
        }
    }

    fn rooted(fixture: &Fixture) -> (HotState, Catalog) {
        cold(&fixture.state, &fixture.catalog)
    }

    /// The same root with a spent Puzzle: the reference suffix.
    fn spent(fixture: &Fixture) -> (HotState, Catalog) {
        let mut state = fixture.state.clone();
        state.fanouts.set_puzzle_armed(false);
        cold(&state, &fixture.catalog)
    }

    /// Answer every later, independent choice with its first legal answer.
    fn finish_first(mut state: HotState, catalog: &Catalog) -> HotState {
        while state.pending.is_some() {
            let action = legal_actions(&state, catalog)[0];
            state = apply_action(&state, catalog, &action).unwrap().state;
        }
        state
    }

    fn assert_parked_on_puzzle(parked: &HotState) {
        assert!(parked.pending.is_some());
        assert!(!parked.fanouts.puzzle_armed());
        assert!(matches!(
            parked.frames.as_slice().first(),
            Some(crate::frame::Frame::ActionReplay { .. })
        ));
        assert!(parked_draw_depth(parked).is_some(), "{:#?}", parked.frames);
    }

    /// Stratagem 2, five Defends in Hand, empty Draw pile. EndTurn discards
    /// the Hand; Toadpole's three-hit Spike Spit (loop 0) lands unblocked and
    /// the Puzzle's first one-card Draw reshuffles five cards against
    /// Stratagem 2: a real selection in the middle of the enemy AttackCommand.
    fn stratagem_enemy_hit() -> Fixture {
        let mut fixture = fixture(&[CardId::DefendSilent], 0);
        let defend = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        for uid in 1..6 {
            fixture
                .state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(uid, defend));
        }
        fixture
    }

    /// #3114 witness: Puzzle Draw parks on a Stratagem reshuffle from the
    /// enemy-attack caller; every answer reaches a terminal with a cold round
    /// trip and admission at every step; the two remaining hits, the move
    /// suffix and the rest of the enemy phase resume exactly once.
    #[test]
    fn stratagem_park_mid_enemy_hit_resumes_the_remaining_hits_once() {
        let fixture = stratagem_enemy_hit();
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        let hit = 50 - parked.hp;
        assert!(hit > 0);
        let (reference_state, reference_catalog) = spent(&fixture);
        // Without the Puzzle the five Defends reshuffle at the next turn's
        // Hand Draw instead; that later Stratagem pick changes no hit.
        let reference = finish_first(
            apply_action(&reference_state, &reference_catalog, &Action::EndTurn)
                .unwrap()
                .state,
            &reference_catalog,
        );
        assert_eq!(reference.hp, 50 - 3 * hit);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20); // P(5, 2) ordered picks.
        let mut hands = std::collections::BTreeSet::new();
        for terminal in &terminals {
            assert_eq!(terminal.hp, reference.hp);
            assert_eq!(terminal.monsters, reference.monsters);
            assert_eq!(terminal.turn, reference.turn);
            assert_eq!(terminal.player_phase, reference.player_phase);
            assert_eq!(terminal.block, reference.block);
            // Two selected + three Puzzle draws; the Hand Draw finds nothing.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 5);
            assert_eq!(terminal.cards_drawn_combat, 3);
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

    /// #3114 witness: a selecting Hellraiser Strike reached through a nested
    /// Pommel Strike Draw, from the enemy-attack caller.
    #[test]
    fn hellraiser_nested_park_mid_enemy_hit_drives_every_answer_once() {
        let mut fixture = fixture(
            &[
                CardId::PommelStrike,
                CardId::SeekerStrike,
                CardId::DefendIronclad,
            ],
            0,
        );
        let (pommel, seeker, defend) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([
                card(1, pommel),
                card(2, seeker),
                card(3, defend),
                card(4, defend),
                card(5, defend),
                card(6, defend),
                card(7, defend),
                card(8, defend),
            ]);
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            matches!(
                &parked.frames.as_slice()[depth + 1..],
                [
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let hit = 50 - parked.hp;
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            // All three Spike Spit hits land exactly once.
            assert_eq!(terminal.hp, 50 - 3 * hit);
            assert!(terminal.turn > state.turn);
            // Pommel and Seeker under the Puzzle, then both again when the
            // next turn's Hand Draw reshuffles them back.
            assert_eq!(terminal.history.card_plays_finished_combat, 4);
        }
    }

    /// #3114 witness: the Thorns caller. A Strike into Thorns 3 takes the
    /// retaliation before its own hit lands; the Puzzle's reshuffle parks
    /// inside that card play and the hit then lands exactly once.
    #[test]
    fn stratagem_park_inside_thorns_lands_the_strike_once() {
        let mut fixture = fixture(&[CardId::StrikeIronclad, CardId::DefendIronclad], 2);
        let (strike, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture.state.monsters_mut()[0]
            .powers
            .set(PowerId::Thorns, SlotWire::Int, 3);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, strike));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: Some(0),
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert_parked_on_puzzle(&parked);
        assert_eq!(parked.hp, 47);
        assert_eq!(parked.monsters[0].hp, 1_000);
        let (reference_state, reference_catalog) = spent(&fixture);
        let reference = apply_action(&reference_state, &reference_catalog, &play)
            .unwrap()
            .state;
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.hp, 47);
            assert_eq!(terminal.monsters, reference.monsters);
            assert_eq!(terminal.energy, reference.energy);
            assert_eq!(
                terminal.history.card_plays_finished_combat,
                reference.history.card_plays_finished_combat
            );
            assert!(
                terminal
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 1)
            );
        }
    }

    /// #3114 witness: a selecting Hellraiser Strike from inside a card play
    /// (the Thorns caller and the card self-damage caller). The inline answer
    /// resumes the parked children above the synchronous plays still running
    /// beneath the Puzzle, and the outer card finishes exactly once.
    #[test]
    fn hellraiser_park_inside_a_card_play_finishes_the_card_once() {
        for thorns in [true, false] {
            let mut fixture = fixture(
                &[
                    CardId::StrikeIronclad,
                    CardId::Bloodletting,
                    CardId::PommelStrike,
                    CardId::SeekerStrike,
                    CardId::DefendIronclad,
                ],
                2,
            );
            let atoms = fixture.atoms.clone();
            fixture
                .state
                .powers
                .set(PowerId::Hellraiser, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(
                &mut fixture.state,
            );
            if thorns {
                fixture.state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Thorns, SlotWire::Int, 3);
            }
            let played = if thorns { atoms[0] } else { atoms[1] };
            fixture
                .state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(1, played));
            fixture
                .state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend([
                    card(2, atoms[2]),
                    card(3, atoms[3]),
                    card(4, atoms[4]),
                    card(5, atoms[4]),
                    card(6, atoms[4]),
                ]);
            let (state, catalog) = rooted(&fixture);
            let play = Action::Play {
                uid: 1,
                target: thorns.then_some(0),
                selection: SelectionRef::NONE,
            };
            let parked = apply_action(&state, &catalog, &play).unwrap().state;
            assert_parked_on_puzzle(&parked);
            let depth = parked_draw_depth(&parked).unwrap();
            assert!(
                matches!(
                    &parked.frames.as_slice()[depth + 1..],
                    [
                        crate::frame::Frame::CardPlay { .. },
                        crate::frame::Frame::Draw { .. },
                        crate::frame::Frame::CardPlay { .. }
                    ]
                ),
                "{:#?}",
                parked.frames
            );
            let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
            assert_eq!(visited, 1);
            assert!(!terminals.is_empty());
            for terminal in &terminals {
                // The played card, Pommel and Seeker each finish once.
                assert_eq!(terminal.history.card_plays_finished_combat, 3);
                assert!(
                    terminal
                        .piles
                        .get(PileId::Discard)
                        .as_slice()
                        .iter()
                        .any(|card| card.uid == 1)
                );
                assert_eq!(terminal.hp, parked.hp);
            }
        }
    }

    /// #3114 witness: the card self-damage caller. Bloodletting's HP loss
    /// fires the Puzzle mid-body; its Energy suffix runs exactly once.
    #[test]
    fn stratagem_park_inside_card_self_damage_finishes_the_body_once() {
        let mut fixture = fixture(&[CardId::Bloodletting, CardId::DefendIronclad], 2);
        let (bloodletting, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, bloodletting));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert_parked_on_puzzle(&parked);
        let (reference_state, reference_catalog) = spent(&fixture);
        let reference = apply_action(&reference_state, &reference_catalog, &play)
            .unwrap()
            .state;
        let (terminals, _) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.hp, reference.hp);
            assert_eq!(terminal.energy, reference.energy);
            assert_eq!(
                terminal.history.card_plays_finished_combat,
                reference.history.card_plays_finished_combat
            );
        }
    }

    /// #3114 witness: the turn-end card caller. Burn's end-of-turn damage in
    /// Hand fires the Puzzle before the Hand is discarded; the Hand flush,
    /// enemy phase and next turn run exactly once after the answer.
    #[test]
    fn stratagem_park_inside_turn_end_burn_resumes_the_turn_once() {
        let mut fixture = fixture(&[CardId::Burn, CardId::DefendSilent], 2);
        let (burn, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, burn));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        assert_eq!(parked.hp, 48);
        let (reference_state, reference_catalog) = spent(&fixture);
        let reference = finish_first(
            apply_action(&reference_state, &reference_catalog, &Action::EndTurn)
                .unwrap()
                .state,
            &reference_catalog,
        );
        let (terminals, _) = every_answer_to_terminal(&parked, &catalog);
        // The next turn's Hand Draw reshuffles again under Stratagem, so
        // every Puzzle answer is followed by that later, ordinary pick.
        assert_eq!(terminals.len(), 20 * 30);
        for terminal in &terminals {
            assert_eq!(terminal.hp, reference.hp);
            assert_eq!(terminal.monsters, reference.monsters);
            assert_eq!(terminal.turn, reference.turn);
            assert_eq!(terminal.player_phase, reference.player_phase);
        }
    }

    /// #3114 witness: the turn-start power tick. Inferno's self-damage at the
    /// start of the next turn, after its Hand Draw, fires the Puzzle inside
    /// the EndTurn action.
    #[test]
    fn stratagem_park_inside_turn_start_inferno_resumes_the_turn_once() {
        let mut fixture = fixture(&[CardId::DefendSilent], 2);
        let defend = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture.state.powers.set(PowerId::Inferno, SlotWire::Int, 1);
        fixture
            .state
            .powers
            .set(PowerId::InfernoSelf, SlotWire::Int, 1);
        assert!(
            fixture
                .state
                .fanouts
                .set_after_player_turn_start_order(&[PowerId::Inferno])
        );
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((1..6).map(|uid| card(uid, defend)));
        // The Hand Draw (before Inferno's tick) takes these five exactly.
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((6..11).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        assert_eq!(parked.hp, 49);
        let (reference_state, reference_catalog) = spent(&fixture);
        let reference = finish_first(
            apply_action(&reference_state, &reference_catalog, &Action::EndTurn)
                .unwrap()
                .state,
            &reference_catalog,
        );
        let (terminals, _) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.hp, reference.hp);
            assert_eq!(terminal.monsters, reference.monsters);
            assert_eq!(terminal.turn, reference.turn);
            assert_eq!(terminal.energy, reference.energy);
        }
    }

    fn swift_defend() -> CardIdentity {
        CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: Some(crate::catalog::CardEnchantment {
                id: crate::ids::EnchantmentId::Swift,
                amount: 2,
            }),
        }
    }

    /// #3115 witness: Swift's Draw parks on a Stratagem reshuffle after the
    /// body. The parked state already projects SWIFT_USED (native sets
    /// `Status` before the await); every answer reaches a terminal with a
    /// cold round trip and admission at every step; the body's Block and the
    /// AfterCardPlayed walk run exactly once.
    #[test]
    fn swift_stratagem_park_projects_spent_status_and_finishes_once() {
        let mut fixture = fixture_with(&[swift_defend(), plain(CardId::DefendIronclad)], &[], 2);
        let (swift, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, swift));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(parked.pending.is_some());
        let depth = parked_draw_depth(&parked).unwrap();
        let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[depth] else {
            unreachable!()
        };
        let draw = parked.frames.draw(record).unwrap();
        assert_eq!(draw.caller, crate::hot::DrawCaller::SwiftEnchantment);
        assert_eq!((draw.requested, draw.completed), (2, 0));
        assert_eq!(parked.card_states.get(1).enchantment_state.get(), Some(1));
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let spent = wire
            .piles
            .values()
            .flatten()
            .find(|row| row.enchantment == Some(serde_json::json!(["SWIFT_USED", 2])));
        assert!(spent.is_some(), "the parked Swift card projects SWIFT_USED");
        assert_eq!(parked.block, 5);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.block, 5);
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
            // Two selected into Hand, then the Swift Draw's two.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 4);
            assert_eq!(terminal.card_states.get(1).enchantment_state.get(), Some(1));
        }
    }

    /// #3115 witness: a selecting Hellraiser Strike reached from Swift's
    /// Draw, through a nested Pommel Strike Draw.
    #[test]
    fn swift_hellraiser_nested_park_finishes_the_swift_card_once() {
        let mut fixture = fixture_with(
            &[
                swift_defend(),
                plain(CardId::PommelStrike),
                plain(CardId::SeekerStrike),
                plain(CardId::DefendIronclad),
            ],
            &[],
            2,
        );
        let atoms = fixture.atoms.clone();
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, atoms[0]));
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend([
                card(2, atoms[1]),
                card(3, atoms[2]),
                card(4, atoms[3]),
                card(5, atoms[3]),
                card(6, atoms[3]),
            ]);
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(parked.pending.is_some());
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            matches!(
                &parked.frames.as_slice()[depth + 1..],
                [
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            // Swift Defend, Pommel and Seeker each finish once.
            assert_eq!(terminal.history.card_plays_finished_combat, 3);
            assert_eq!(terminal.block, 5);
            assert_eq!(terminal.card_states.get(1).enchantment_state.get(), Some(1));
        }
    }

    /// #3115 witness: a spent Swift (Status already set) returns before its
    /// Draw at IL_001d-0025, so a second play under Stratagem never parks.
    #[test]
    fn spent_swift_never_parks() {
        let mut fixture = fixture_with(&[swift_defend(), plain(CardId::DefendIronclad)], &[], 2);
        let (swift, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, swift));
        let mut instance = fixture.state.card_states.get(1);
        instance.enchantment_state =
            crate::hot::OptionalNonNegativeI32::from_option(Some(1)).unwrap();
        fixture.state.card_states.set(1, instance);
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let played = apply_action(
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
        assert!(played.pending.is_none());
        assert_eq!(played.piles.get(PileId::Discard).len(), 6);
    }

    /// #3115 witness: a replayed Swift card. Burst replays the Skill body,
    /// and each replay runs OnPlay; the Status published before the first
    /// Draw gates the second, so the card draws its Amount exactly once even
    /// when that first Draw parks and the replay continues after the answer.
    #[test]
    fn replayed_swift_draws_once_across_a_park() {
        let mut fixture = fixture_with(&[swift_defend(), plain(CardId::DefendIronclad)], &[], 2);
        let (swift, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture.state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, swift));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(parked.pending.is_some());
        let (terminals, _) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            // Both bodies' Block; one Swift Draw of two after the two picks.
            assert_eq!(terminal.block, 10);
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 4);
            assert_eq!(terminal.powers.value(PowerId::Burst), 0);
        }
    }

    /// A forged Swift owner refuses: a Draw record whose `requested` is not
    /// the Amount of any spent Swift card, or a Swift not yet spent.
    #[test]
    fn forged_swift_owner_records_refuse() {
        let mut fixture = fixture_with(&[swift_defend(), plain(CardId::DefendIronclad)], &[], 2);
        let (swift, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, swift));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
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
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let depth = parked_draw_depth(&parked).unwrap();
        let mut widened = wire.clone();
        widened.continuations[depth]
            .fields
            .insert("requested".to_owned(), serde_json::json!(3));
        assert!(HotBoundary::from_canonical(&widened, &loaded_catalog).is_err());

        let mut unspent = parked.clone();
        let mut instance = unspent.card_states.get(1);
        instance.enchantment_state = crate::hot::OptionalNonNegativeI32::from_option(None).unwrap();
        unspent.card_states.set(1, instance);
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&unspent, &loaded_catalog)
                .is_err()
        );
    }

    /// Walk one answer path (`pick` chooses among the legal answers) from
    /// `parked` to a state with no pending choice, round-tripping and
    /// admitting every parked state. Returns the terminal and the number of
    /// parked states on the path.
    fn answer_path(
        parked: &HotState,
        catalog: &Catalog,
        pick: impl Fn(usize) -> usize,
    ) -> (HotState, usize) {
        let mut state = parked.clone();
        let mut parks = 0;
        while state.pending.is_some() {
            parks += 1;
            assert!(parks < 200, "unbounded answer path");
            let (loaded, loaded_catalog) = cold(&state, catalog);
            assert_eq!(loaded, state);
            let actions = legal_actions(&loaded, &loaded_catalog);
            assert!(!actions.is_empty());
            let action = actions[pick(actions.len())];
            state = apply_action(&loaded, &loaded_catalog, &action)
                .unwrap_or_else(|error| panic!("{action:?}: {error:?}"))
                .state;
        }
        assert!(state.frames.is_empty());
        cold(&state, catalog);
        (state, parks)
    }

    /// #3114 witness (review B1): a selecting Hellraiser Strike AutoPlayed
    /// DIRECTLY by a Puzzle Draw, so the Seeker's CardPlay sits on the
    /// `centennial_puzzle` Draw frame. Three Seekers make the Puzzle park once
    /// per Draw: later parks re-execute with earlier tape answers consumed
    /// in place as non-last answers.
    #[test]
    fn hellraiser_seeker_directly_under_the_puzzle_draw_loads_cold() {
        let mut fixture = fixture(&[CardId::SeekerStrike, CardId::DefendIronclad], 0);
        let (seeker, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.extend((1..4).map(|uid| card(uid, seeker)));
        draw.extend((4..12).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            matches!(
                &parked.frames.as_slice()[depth + 1..],
                [crate::frame::Frame::CardPlay { .. }]
            ),
            "{:#?}",
            parked.frames
        );
        let hit = 50 - parked.hp;
        let _ = super::take_branch_hits();
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        let hits = super::take_branch_hits();
        assert!(visited > 1, "the Puzzle parks once per Seeker Draw");
        assert!(hits.inline_non_last > 0, "{hits:?}");
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            assert_eq!(terminal.hp, 50 - 3 * hit);
            assert!(terminal.turn > state.turn);
        }
    }

    /// #3115 witness (review B1): Swift's Draw AutoPlays a selecting Seeker
    /// directly, so the Seeker sits on the `swift_enchantment` Draw frame.
    #[test]
    fn hellraiser_seeker_directly_under_the_swift_draw_loads_cold() {
        let mut fixture = fixture_with(
            &[
                swift_defend(),
                plain(CardId::SeekerStrike),
                plain(CardId::DefendIronclad),
            ],
            &[],
            2,
        );
        let atoms = fixture.atoms.clone();
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, atoms[0]));
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.extend([card(2, atoms[1]), card(3, atoms[1])]);
        draw.extend((4..9).map(|uid| card(uid, atoms[2])));
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(parked.pending.is_some());
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            matches!(
                &parked.frames.as_slice()[depth + 1..],
                [crate::frame::Frame::CardPlay { .. }]
            ),
            "{:#?}",
            parked.frames
        );
        let _ = super::take_branch_hits();
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        let hits = super::take_branch_hits();
        // Swift 2 draws both Seekers: the second parks after the first's
        // answer is consumed in place.
        assert!(visited > 1);
        assert!(hits.inline_non_last > 0, "{hits:?}");
        for terminal in &terminals {
            assert_eq!(terminal.block, 5);
            assert_eq!(terminal.history.card_plays_finished_combat, 3);
            assert_eq!(terminal.card_states.get(1).enchantment_state.get(), Some(1));
        }
    }

    /// Review B2 witness: an ORDINARY park before a receipt-owned one in the
    /// same action. The next turn's Hand Draw reshuffles against Stratagem 2
    /// (a turn-start-owned park, answered as a public Select); Inferno's
    /// turn-start tick then fires the Puzzle, whose Draw AutoPlays a selecting
    /// Seeker. Answering that Puzzle park re-executes EndTurn and must apply
    /// the first answer through the driver's public-Select loop.
    #[test]
    fn an_ordinary_park_before_the_puzzle_park_replays_through_public_select() {
        // Toadpole's single-hit Whirl lands on 20 Block: no unblocked damage
        // before the next turn, so the Puzzle is still armed at turn start.
        let mut fixture = fixture(&[CardId::SeekerStrike, CardId::DefendIronclad], 1);
        fixture.state.block = 20;
        let (seeker, defend) = (fixture.atoms[0], fixture.atoms[1]);
        for (power, amount) in [
            (PowerId::Hellraiser, 1),
            (PowerId::Stratagem, 2),
            (PowerId::Inferno, 1),
            (PowerId::InfernoSelf, 1),
        ] {
            fixture.state.powers.set(power, SlotWire::Int, amount);
        }
        assert!(
            fixture
                .state
                .fanouts
                .set_after_player_turn_start_order(&[PowerId::Inferno])
        );
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        // Three Defends to draw, then the discarded Hand (two Seekers, five
        // Defends) reshuffles mid Hand Draw. Found by enumerating small decks
        // under the fixture's fixed RNG words.
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..4).map(|uid| card(uid, defend)));
        let hand = fixture.state.piles.get_mut(PileId::Hand).make_mut();
        hand.extend((4..6).map(|uid| card(uid, seeker)));
        hand.extend((6..11).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let first = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(
            first
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&first.frames)
                .is_some()
        );
        assert!(parked_draw_depth(&first).is_none());
        assert!(first.fanouts.puzzle_armed());
        let (loaded, loaded_catalog) = cold(&first, &catalog);
        let mut reached = 0;
        for action in legal_actions(&loaded, &loaded_catalog) {
            let second = apply_action(&loaded, &loaded_catalog, &action)
                .unwrap()
                .state;
            // Only answers that leave a Seeker on top when the Puzzle draws
            // reach a receipt-owned park.
            if second.pending.is_none() || parked_draw_depth(&second).is_none() {
                continue;
            }
            assert_parked_on_puzzle(&second);
            reached += 1;
            let _ = super::take_branch_hits();
            let (terminals, _) = every_answer_to_terminal(&second, &catalog);
            let hits = super::take_branch_hits();
            assert!(hits.driver_selects > 0, "{hits:?}");
            assert!(!terminals.is_empty());
            for terminal in &terminals {
                assert!(!terminal.fanouts.puzzle_armed());
                assert_eq!(terminal.hp, 49);
                assert_eq!(terminal.turn, first.turn);
            }
        }
        assert!(reached > 0, "some Hand Draw answer reaches the Puzzle park");
    }

    /// Review B2 witness: one Puzzle Draw that parks twice. Stratagem's
    /// reshuffle parks first; after its answer the same Draw draws a Seeker,
    /// which Hellraiser AutoPlays and which parks again before the Draw
    /// returns (the `continue` arm of `receipt_owned_draw`).
    #[test]
    fn one_puzzle_draw_parking_twice_takes_the_continue_arm() {
        let mut fixture = fixture(&[CardId::SeekerStrike], 1);
        let seeker = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..7).map(|uid| card(uid, seeker)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        let _ = super::take_branch_hits();
        let (first_terminal, first_parks) = answer_path(&parked, &catalog, |_| 0);
        let (last_terminal, last_parks) = answer_path(&parked, &catalog, |n| n - 1);
        let hits = super::take_branch_hits();
        assert!(hits.inline_continue > 0, "{hits:?}");
        assert!(hits.inline_non_last > 0, "{hits:?}");
        assert!(first_parks > 1 && last_parks > 1);
        for terminal in [&first_terminal, &last_terminal] {
            assert!(!terminal.fanouts.puzzle_armed());
            assert!(terminal.turn > state.turn);
        }
    }

    /// #3172 witness (census f86e840dc69730f5, f8ff1bd93517512d,
    /// fe81e09bdf26b01e): Bloodletting's `hp_loss` step outside an
    /// ActionReplay used the catalogless card-damage entry and refused
    /// "Centennial Puzzle card damage requires catalog". The step now keeps
    /// its catalog, so the self-damage's `Hook.AfterDamageReceived` walk runs
    /// the Puzzle's three one-card Draws (`0x321758` IL_007f-0116)
    /// synchronously before the body's Energy step, exactly as the
    /// spent-Puzzle reference plus three draws.
    #[test]
    fn bloodletting_self_damage_runs_the_puzzle_draw_with_the_catalog() {
        let mut fixture = fixture(&[CardId::Bloodletting, CardId::DefendIronclad], 0);
        let (bloodletting, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, bloodletting));
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..6).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let done = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(done.pending.is_none());
        assert!(!done.fanouts.puzzle_armed());
        assert_eq!(done.hp, 47);
        assert_eq!(done.energy, state.energy + 2);
        assert_eq!(done.piles.get(PileId::Hand).len(), 3);
        assert_eq!(done.piles.get(PileId::Draw).len(), 1);
        assert_eq!(done.cards_drawn_combat, 3);
        cold(&done, &catalog);

        let (reference_state, reference_catalog) = spent(&fixture);
        let reference = apply_action(&reference_state, &reference_catalog, &play)
            .unwrap()
            .state;
        assert_eq!(reference.hp, 47);
        assert_eq!(reference.energy, done.energy);
        assert!(reference.piles.get(PileId::Hand).is_empty());
    }

    /// With neither Hellraiser nor Stratagem live the Puzzle keeps the
    /// synchronous command Draw and the action never parks.
    #[test]
    fn no_blocking_listener_keeps_the_synchronous_puzzle_draw() {
        let mut fixture = stratagem_enemy_hit();
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 0);
        let (state, catalog) = rooted(&fixture);
        let done = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(done.pending.is_none());
        assert!(done.frames.is_empty());
        assert!(!done.fanouts.puzzle_armed());
    }

    /// Forged Puzzle-owner records refuse at load: the receipt removed, a
    /// widened `requested`, a re-armed Puzzle, and a forged receipt answer.
    #[test]
    fn forged_puzzle_owner_records_refuse() {
        let fixture = stratagem_enemy_hit();
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let depth = parked_draw_depth(&parked).unwrap();

        let mut rootless = wire.clone();
        rootless.continuations.remove(0);
        assert!(
            HotBoundary::catalog_from_canonical(&rootless)
                .and_then(|catalog| HotBoundary::from_canonical(&rootless, &catalog))
                .is_err()
        );

        let mut widened = wire.clone();
        widened.continuations[depth]
            .fields
            .insert("requested".to_owned(), serde_json::json!(3));
        assert!(HotBoundary::from_canonical(&widened, &loaded_catalog).is_err());

        let mut recaller = wire.clone();
        recaller.continuations[depth]
            .fields
            .insert("caller".to_owned(), serde_json::json!("none"));
        assert!(HotBoundary::from_canonical(&recaller, &loaded_catalog).is_err());

        let mut rearmed = parked.clone();
        rearmed.fanouts.set_puzzle_armed(true);
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&rearmed, &loaded_catalog)
                .is_err()
        );
    }

    /// The carrier publishes only its own park, and never a swallowed one.
    #[test]
    fn carrier_settle_publishes_only_an_unswallowed_park() {
        let fixture = stratagem_enemy_hit();
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let mut rootless = parked.clone();
        let depth = parked_draw_depth(&parked).unwrap();
        let _ = depth;
        // A park outside any carrier is a plain refusal.
        assert_eq!(
            super::park(&rootless),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        // A park the carrier sees unwind is published.
        {
            let guard = CarrierGuard::enter();
            assert_eq!(
                super::park(&parked),
                Err(EngineRefusal::MalformedArgs(PARK_SITE))
            );
            let mut next = state.clone();
            assert_eq!(
                guard.settle(&mut next, Err(EngineRefusal::MalformedArgs(PARK_SITE))),
                Ok(())
            );
            assert_eq!(next, parked);
        }
        // A swallowed park (the action returned Ok) refuses.
        {
            let guard = CarrierGuard::enter();
            let _ = super::park(&parked);
            assert_eq!(
                guard.settle(&mut rootless, Ok(())),
                Err(EngineRefusal::ContinuationNotModeled)
            );
        }
        // A rewritten park (another refusal after it) refuses.
        {
            let guard = CarrierGuard::enter();
            let _ = super::park(&parked);
            assert_eq!(
                guard.settle(&mut rootless, Err(EngineRefusal::CombatOver)),
                Err(EngineRefusal::ContinuationNotModeled)
            );
        }
        // The park signal with no snapshot refuses.
        {
            let guard = CarrierGuard::enter();
            assert_eq!(
                guard.settle(&mut rootless, Err(EngineRefusal::MalformedArgs(PARK_SITE))),
                Err(EngineRefusal::ContinuationNotModeled)
            );
        }
    }

    // ---- #3201: Joss Paper's threshold Draw -------------------------------

    /// A Toadpole fight holding Joss Paper at `CardsExhausted == 4`, so the
    /// next non-Ethereal exhaust (or the side-end fold of one Ethereal
    /// exhaust) issues a one-card threshold Draw.
    fn joss(cards: &[CardId], loop_pos: i32) -> Fixture {
        let identities = cards.iter().map(|id| plain(*id)).collect::<Vec<_>>();
        let mut fixture = fixture_with(&identities, &[RelicId::RelicJossPaper], loop_pos);
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
        fixture
    }

    /// The same root without Joss Paper: the reference suffix.
    fn without_joss(fixture: &Fixture, cards: &[CardId]) -> (HotState, Catalog) {
        let identities = cards.iter().map(|id| plain(*id)).collect::<Vec<_>>();
        let bare = fixture_with(&identities, &[], 0);
        assert_eq!(bare.atoms, fixture.atoms);
        let mut state = fixture.state.clone();
        assert!(state.fanouts.set_joss_paper_cards_exhausted(-1));
        cold(&state, &bare.catalog)
    }

    /// The parked receipt-owned Draw's `(requested, completed)`, asserting
    /// it is Joss Paper's.
    fn parked_joss_draw(parked: &HotState) -> (u32, u32) {
        assert!(parked.pending.is_some());
        assert!(matches!(
            parked.frames.as_slice().first(),
            Some(crate::frame::Frame::ActionReplay { .. })
        ));
        let depth = parked_draw_depth(parked).expect("a receipt-owned Draw");
        let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[depth] else {
            unreachable!()
        };
        let draw = parked.frames.draw(record).unwrap();
        assert_eq!(draw.caller, crate::hot::DrawCaller::JossPaper);
        (draw.requested, draw.completed)
    }

    const PLAY_IMPERVIOUS: Action = Action::Play {
        uid: 1,
        target: None,
        selection: SelectionRef::NONE,
    };

    /// Impervious (Block 30, Exhaust) in Hand, five Defends in Discard, Stratagem 2.
    fn joss_stratagem_exhaust() -> Fixture {
        let mut fixture = joss(&[CardId::Impervious, CardId::DefendIronclad], 2);
        let (impervious, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, impervious));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        fixture
    }

    /// #3201 witness, `AfterCardExhausted` caller (`0x3275a0` IL_004f-00bd):
    /// the fifth exhaust's Draw reshuffles five Defends against Stratagem 2
    /// and parks inside the card play. The parked counter still holds 5,
    /// because `%= 5` (`0x327888` IL_00ec-0109) follows the Draw; every
    /// answer then draws the one card, writes the remainder once and
    /// finishes the play once.
    #[test]
    fn joss_paper_exhaust_draw_parks_on_stratagem_and_finishes_once() {
        let fixture = joss_stratagem_exhaust();
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let parked = apply_action(&state, &catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        assert_eq!(parked_joss_draw(&parked), (1, 0));
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        assert_eq!(parked.fanouts.joss_paper_cards_exhausted(), 5);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20); // P(5, 2) ordered picks.
        let mut hands = std::collections::BTreeSet::new();
        for terminal in &terminals {
            assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
            // Two selected by Stratagem plus the one Joss Paper card.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 3);
            assert_eq!(terminal.cards_drawn_combat, 1);
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
            assert_eq!(terminal.energy, state.energy - 2);
            assert!(
                terminal
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 1)
            );
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

    /// #3201 witness: below the threshold Joss Paper issues no Draw, so the
    /// same play neither parks nor draws; the counter just advances.
    #[test]
    fn joss_paper_below_threshold_never_parks() {
        let mut fixture = joss_stratagem_exhaust();
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(3));
        let (state, catalog) = rooted(&fixture);
        let done = apply_action(&state, &catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        assert!(done.pending.is_none());
        assert!(done.frames.is_empty());
        assert_eq!(done.fanouts.joss_paper_cards_exhausted(), 4);
        assert!(done.piles.get(PileId::Hand).is_empty());
    }

    /// #3201 witness: with neither Hellraiser nor Stratagem live the Draw
    /// keeps the certified synchronous command (no Draw frame, no park), and
    /// the result is the Joss-free play plus exactly one drawn card.
    #[test]
    fn joss_paper_without_a_blocking_listener_keeps_the_synchronous_draw() {
        let mut fixture = joss_stratagem_exhaust();
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 0);
        let (state, catalog) = rooted(&fixture);
        let done = apply_action(&state, &catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        assert!(done.pending.is_none());
        assert!(done.frames.is_empty());
        assert_eq!(done.fanouts.joss_paper_cards_exhausted(), 0);
        assert_eq!(done.piles.get(PileId::Hand).len(), 1);
        assert_eq!(done.cards_drawn_combat, 1);
        let (reference_state, reference_catalog) =
            without_joss(&fixture, &[CardId::Impervious, CardId::DefendIronclad]);
        let reference = apply_action(&reference_state, &reference_catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        assert_eq!(reference.cards_drawn_combat, 0);
        assert_eq!(done.energy, reference.energy);
        assert_eq!(done.monsters, reference.monsters);
    }

    /// #3201 witness: a selecting Hellraiser Strike AutoPlayed directly by
    /// Joss Paper's Draw, so the Seeker's CardPlay sits on the `joss_paper`
    /// Draw frame inside Impervious's play. Every answer finishes both plays
    /// once and writes the counter remainder once.
    #[test]
    fn joss_paper_hellraiser_seeker_directly_under_the_draw_loads_cold() {
        let cards = [
            CardId::Impervious,
            CardId::SeekerStrike,
            CardId::DefendIronclad,
        ];
        let mut fixture = joss(&cards, 2);
        let (impervious, seeker, defend) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, impervious));
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.push(card(2, seeker));
        draw.extend((3..8).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let parked = apply_action(&state, &catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        assert_eq!(parked_joss_draw(&parked), (1, 0));
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            matches!(
                &parked.frames.as_slice()[depth + 1..],
                [crate::frame::Frame::CardPlay { .. }]
            ),
            "{:#?}",
            parked.frames
        );
        assert_eq!(parked.fanouts.joss_paper_cards_exhausted(), 5);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
            // Impervious and the AutoPlayed Seeker each finish once.
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
            assert_eq!(terminal.energy, state.energy - 2);
            assert!(terminal.monsters[0].hp < 1_000);
        }
    }

    /// #3201 witness, `AfterSideTurnEnd` caller (`0x3276ac` IL_003a-00b2):
    /// Dazed exhausts at turn end with `causedByEthereal`, which only
    /// tallies `EtherealCount` (`0x3275a0` IL_0035-004d). The side-end fold
    /// makes `CardsExhausted` 5 and its Draw reshuffles the flushed Hand
    /// against Stratagem 2 mid-EndTurn; the enemy phase and the next turn
    /// run exactly once after every answer.
    #[test]
    fn joss_paper_side_end_draw_parks_on_stratagem_and_resumes_the_turn_once() {
        let cards = [CardId::Dazed, CardId::DefendIronclad];
        let mut fixture = joss(&cards, 2);
        let (dazed, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        let hand = fixture.state.piles.get_mut(PileId::Hand).make_mut();
        hand.push(card(1, dazed));
        hand.extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(parked_joss_draw(&parked), (1, 0));
        assert_eq!(parked.fanouts.joss_paper_ethereal_count(), 0);
        assert_eq!(parked.fanouts.joss_paper_cards_exhausted(), 5);
        assert_eq!(parked.turn, state.turn);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20);
        let turn = terminals[0].turn;
        assert!(turn > state.turn);
        for terminal in &terminals {
            assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
            assert_eq!(terminal.fanouts.joss_paper_ethereal_count(), 0);
            assert_eq!(terminal.turn, turn);
            assert_eq!(terminal.hp, terminals[0].hp);
            assert_eq!(terminal.monsters, terminals[0].monsters);
            assert!(
                terminal
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 1)
            );
        }
    }

    /// Forged Joss Paper owner records refuse: a rewritten caller, the relic
    /// missing from the catalog, a disarmed counter, and a Draw already
    /// complete.
    #[test]
    fn forged_joss_paper_owner_records_refuse() {
        let fixture = joss_stratagem_exhaust();
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &PLAY_IMPERVIOUS)
            .unwrap()
            .state;
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let depth = parked_draw_depth(&parked).unwrap();
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&parked, &loaded_catalog)
                .is_ok()
        );

        let mut recaller = wire.clone();
        recaller.continuations[depth]
            .fields
            .insert("caller".to_owned(), serde_json::json!("none"));
        assert!(HotBoundary::from_canonical(&recaller, &loaded_catalog).is_err());

        let bare = fixture_with(
            &[plain(CardId::Impervious), plain(CardId::DefendIronclad)],
            &[],
            2,
        );
        assert_eq!(bare.atoms, fixture.atoms);
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&parked, &bare.catalog)
                .is_err()
        );

        let mut disarmed = parked.clone();
        assert!(disarmed.fanouts.set_joss_paper_cards_exhausted(-1));
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&disarmed, &loaded_catalog)
                .is_err()
        );

        let mut completed = wire.clone();
        completed.continuations[depth]
            .fields
            .insert("completed".to_owned(), serde_json::json!(1));
        assert!(HotBoundary::from_canonical(&completed, &loaded_catalog).is_err());

        // More cards than the parked counter can have issued.
        let mut widened = wire.clone();
        widened.continuations[depth]
            .fields
            .insert("requested".to_owned(), serde_json::json!(2));
        assert!(HotBoundary::from_canonical(&widened, &loaded_catalog).is_err());
    }

    // ---- #3386: deferred-choice hook listeners ------------------------------

    /// Canonical round trip without admission, for roots the gate refuses.
    fn loaded(fixture: &Fixture) -> (HotState, Catalog) {
        let wire = HotBoundary::try_to_canonical(&fixture.state, &fixture.catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        (state, catalog)
    }

    fn refuses_at_admission(state: &HotState, catalog: &Catalog, wall: &'static str) -> bool {
        let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
        admit(&wire, state, catalog)
            .is_err_and(|refusal| refusal.contains(MissingCapability::ArgumentShape(wall)))
    }

    fn end_turn_refusal(state: &HotState, catalog: &Catalog) -> EngineRefusal {
        apply_action(state, catalog, &Action::EndTurn)
            .map(|_| ())
            .unwrap_err()
    }

    /// Joss Paper at 4 with one Dazed and five Defends in Hand and Stratagem
    /// 2: the side-end fold of Dazed's Ethereal exhaust makes it 5, and the
    /// threshold Draw reshuffles the flushed Hand into a Stratagem selection
    /// (the #3201 side-end witness), with `extra` relics after Joss Paper.
    fn joss_side_end(extra: &[RelicId], hellraiser: bool) -> Fixture {
        let identities = [plain(CardId::Dazed), plain(CardId::DefendIronclad)];
        let mut relics = vec![RelicId::RelicJossPaper];
        relics.extend_from_slice(extra);
        let mut fixture = fixture_with(&identities, &relics, 2);
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
        let (dazed, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        if hellraiser {
            fixture
                .state
                .powers
                .set(PowerId::Hellraiser, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(
                &mut fixture.state,
            );
        }
        let hand = fixture.state.piles.get_mut(PileId::Hand).make_mut();
        hand.push(card(1, dazed));
        hand.extend((2..7).map(|uid| card(uid, defend)));
        fixture
    }

    /// #3386 witness, the non-commuting side: Parrying Shield is a later
    /// ordinary `AfterSideTurnEnd` listener (a Block read, an RNG target roll
    /// and monster damage) that native runs before the Stratagem choice. The
    /// root refuses at admission, and the Draw refuses by name at runtime.
    #[test]
    fn joss_paper_side_end_draw_refuses_before_a_later_parrying_shield() {
        let fixture = joss_side_end(&[RelicId::RelicParryingShield], false);
        let (state, catalog) = loaded(&fixture);
        assert!(refuses_at_admission(
            &state,
            &catalog,
            super::JOSS_SIDE_END_WALL
        ));
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::JOSS_SIDE_END_WALL)
        );
    }

    /// #3386 witness: with Hellraiser live the continuation can play an
    /// Attack, whose `AfterCardPlayed` writes the flag Art of War's later
    /// side-end rollover reads. Refused at admission and at runtime.
    #[test]
    fn joss_paper_side_end_draw_with_hellraiser_refuses_before_art_of_war() {
        let fixture = joss_side_end(&[RelicId::RelicArtOfWar], true);
        let (state, catalog) = loaded(&fixture);
        assert!(refuses_at_admission(
            &state,
            &catalog,
            super::JOSS_SIDE_END_WALL
        ));
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::JOSS_SIDE_END_WALL)
        );
        // A Hellraiser card the catalog reaches (as a generation closure
        // does) is left to the runtime refusal until it is a physical card.
        let identities = [
            plain(CardId::Dazed),
            plain(CardId::DefendIronclad),
            plain(CardId::Hellraiser),
        ];
        let mut reached = fixture_with(
            &identities,
            &[RelicId::RelicJossPaper, RelicId::RelicArtOfWar],
            2,
        );
        reached
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        assert!(reached.state.fanouts.set_joss_paper_cards_exhausted(4));
        let hellraiser_card = reached.atoms[2];
        let hand = reached.state.piles.get_mut(PileId::Hand).make_mut();
        hand.extend((2..7).map(|uid| card(uid, reached.atoms[1])));
        assert!(!refuses_at_admission(
            &reached.state,
            &reached.catalog,
            super::JOSS_SIDE_END_WALL
        ));
        reached
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(9, hellraiser_card));
        assert!(refuses_at_admission(
            &reached.state,
            &reached.catalog,
            super::JOSS_SIDE_END_WALL
        ));
        // The admission gate's live-Oblivion clause, with no later relic.
        let mut oblivion = joss_side_end(&[], true);
        let (state, catalog) = loaded(&oblivion);
        assert!(!refuses_at_admission(
            &state,
            &catalog,
            super::JOSS_SIDE_END_WALL
        ));
        oblivion.state.monsters_mut()[0]
            .powers
            .set(PowerId::Oblivion, SlotWire::Int, 1);
        assert!(refuses_at_admission(
            &oblivion.state,
            &oblivion.catalog,
            super::JOSS_SIDE_END_WALL
        ));
    }

    /// #3386 witness, the commuting side: Art of War and Kusarigama are later
    /// ordinary side-end listeners, but with Hellraiser absent nothing in the
    /// Stratagem choice or the rest of the Draw plays a card, so their
    /// rollovers commute with it. The root admits, the Draw parks, and every
    /// answer ends with both rollovers applied exactly once.
    #[test]
    fn joss_paper_side_end_draw_parks_beside_commuting_later_listeners() {
        let mut fixture = joss_side_end(&[RelicId::RelicArtOfWar, RelicId::RelicKusarigama], false);
        fixture.state.set_art_of_war_current_attack(true);
        assert!(fixture.state.fanouts.set_kusarigama(2));
        assert_eq!(
            super::super::relics::joss_paper_side_end_draw_wall(&fixture.catalog, &fixture.state),
            None
        );
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(parked_joss_draw(&parked), (1, 0));
        // The receipt carrier holds both rollovers until after the answer.
        assert!(parked.art_of_war_current_attack());
        assert_eq!(parked.fanouts.kusarigama(), 2);

        // The reference: the same turn with no Joss Draw at all. Art of War's
        // rollover shows at the next turn's energy reset (no bonus after an
        // Attack turn), and without the Attack flag the bonus is paid.
        let reference = |attacked: bool| {
            let mut quiet = fixture.state.clone();
            assert!(quiet.fanouts.set_joss_paper_cards_exhausted(0));
            quiet.set_art_of_war_current_attack(attacked);
            let (quiet, quiet_catalog) = cold(&quiet, &fixture.catalog);
            finish_first(
                apply_action(&quiet, &quiet_catalog, &Action::EndTurn)
                    .unwrap()
                    .state,
                &quiet_catalog,
            )
        };
        let (attacked, idle) = (reference(true), reference(false));
        assert_eq!(idle.energy, attacked.energy + 1);
        assert_eq!(attacked.fanouts.kusarigama(), 0);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 20);
        for terminal in &terminals {
            assert_eq!(terminal.turn, attacked.turn);
            assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
            assert_eq!(terminal.energy, attacked.energy);
            assert!(!terminal.art_of_war_current_attack());
            assert_eq!(terminal.fanouts.kusarigama(), 0);
            assert_eq!(terminal.hp, attacked.hp);
            assert_eq!(terminal.monsters, attacked.monsters);
        }
    }

    /// #3386 witness, one row per branch of
    /// `relics::joss_paper_side_end_draw_wall`.
    #[test]
    fn joss_paper_side_end_draw_wall_branches() {
        let wall = Some(super::JOSS_SIDE_END_WALL);
        let catalog_with = |relics: &[RelicId]| {
            let mut all = vec![RelicId::RelicJossPaper];
            all.extend_from_slice(relics);
            fixture_with(&[plain(CardId::DefendIronclad)], &all, 0)
        };
        let check = |relics: &[RelicId], edit: &dyn Fn(&mut HotState)| {
            let mut fixture = catalog_with(relics);
            edit(&mut fixture.state);
            super::super::relics::joss_paper_side_end_draw_wall(&fixture.catalog, &fixture.state)
        };
        let nothing = |_: &mut HotState| {};
        let hellraiser = |state: &mut HotState| {
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        };
        let oblivion = |state: &mut HotState| {
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Oblivion, SlotWire::Int, 1);
        };
        let hellraiser_oblivion = |state: &mut HotState| {
            hellraiser(state);
            oblivion(state);
        };
        // No later listener acts.
        assert_eq!(check(&[], &nothing), None);
        assert_eq!(check(&[], &hellraiser), None);
        // Always refused.
        assert_eq!(check(&[RelicId::RelicLunarPastry], &nothing), wall);
        assert_eq!(check(&[RelicId::RelicParryingShield], &nothing), wall);
        assert_eq!(
            check(&[], &|state: &mut HotState| state.multiplayer_ally_key = 1),
            wall
        );
        // Commuting only while no card can be played.
        assert_eq!(check(&[RelicId::RelicArtOfWar], &nothing), None);
        assert_eq!(check(&[RelicId::RelicArtOfWar], &hellraiser), wall);
        assert_eq!(check(&[RelicId::RelicKusarigama], &nothing), None);
        assert_eq!(check(&[RelicId::RelicKusarigama], &hellraiser), wall);
        assert_eq!(check(&[], &oblivion), None);
        assert_eq!(check(&[], &hellraiser_oblivion), wall);
        // A choice already parked below the Draw.
        let parked = apply_action(
            &rooted(&joss_stratagem_exhaust()).0,
            &rooted(&joss_stratagem_exhaust()).1,
            &PLAY_IMPERVIOUS,
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some());
        assert_eq!(
            super::super::relics::joss_paper_side_end_draw_wall(
                &joss_stratagem_exhaust().catalog,
                &parked
            ),
            wall
        );
    }

    /// #3386 witness: Pael's Eye's `BeforeSideTurnEndEarly` Exhaust feeds
    /// Joss Paper's fifth exhaust, whose Draw reshuffles five Defends against
    /// Stratagem 2. Native starts every later Early and ordinary
    /// BeforeSideTurnEnd listener before that choice, so the root refuses and
    /// the Draw refuses by name.
    #[test]
    fn joss_paper_draw_under_paels_eye_refuses() {
        let identities = [plain(CardId::DefendIronclad)];
        let mut fixture = fixture_with(
            &identities,
            &[RelicId::RelicPaelsEye, RelicId::RelicJossPaper],
            2,
        );
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
        fixture
            .state
            .fanouts
            .set_paels_eye_was_owner_part_last_player_turn(true);
        let defend = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, defend));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = loaded(&fixture);
        assert!(refuses_at_admission(
            &state,
            &catalog,
            super::PAELS_EYE_JOSS_WALL
        ));
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::PAELS_EYE_JOSS_WALL)
        );
    }

    /// #3386 witness, the commuting Late pass: in a Knowledge Demon fight
    /// (counter 1, Disintegration 6), Disintegration's `AfterSideTurnEndLate`
    /// damage fires the Centennial Puzzle, whose Draw reshuffles the flushed
    /// Hand against Stratagem 2. Disintegration is the pass's only listener
    /// acting at the player side end and the pass `WhenAll`s before
    /// SwitchSides, so the park is published and the enemy phase runs once
    /// after every answer.
    #[test]
    fn disintegration_puzzle_draw_parks_in_the_late_pass() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain(CardId::DefendSilent))
            .unwrap();
        builder.intern_monster(MonsterKind::KnowledgeDemon).unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .set_relics(&[RelicId::RelicCentennialPuzzle])
            .unwrap();
        let catalog = builder.build();
        let mut state = fixture_with(&[plain(CardId::DefendSilent)], &[], 0).state;
        state.fanouts.set_puzzle_armed(true);
        let mut demon = HotMonster::new(MonsterKind::KnowledgeDemon, 399);
        demon.max_hp = 399;
        demon.loop_pos = 1;
        assert!(demon.set_knowledge_demon_curse_counter(1));
        state.monsters = std::sync::Arc::new(vec![demon]);
        state.powers.set(PowerId::Disintegration, SlotWire::Int, 6);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 2);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..6).map(|uid| card(uid, defend)));
        assert!(crate::engine::turn::knowledge_demon_state_is_exact(&state));
        let fixture = Fixture {
            state,
            catalog,
            atoms: vec![defend],
        };
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_parked_on_puzzle(&parked);
        assert_eq!(parked.hp, 44, "only Disintegration has hit");
        assert_eq!(parked.turn, state.turn);
        let (loaded_parked, loaded_catalog) = cold(&parked, &catalog);
        let actions = legal_actions(&loaded_parked, &loaded_catalog);
        assert_eq!(actions.len(), 20); // P(5, 2) ordered picks.
        let terminals = actions
            .into_iter()
            .map(|action| {
                let next = apply_action(&loaded_parked, &loaded_catalog, &action)
                    .unwrap_or_else(|error| panic!("{action:?}: {error:?}"))
                    .state;
                // Past the park the next choice is the enemy phase's own.
                assert!(parked_draw_depth(&next).is_none());
                next
            })
            .collect::<Vec<_>>();
        for terminal in &terminals {
            assert_eq!(terminal.hp, terminals[0].hp);
            assert_eq!(terminal.monsters, terminals[0].monsters);
            assert_eq!(terminal.turn, terminals[0].turn);
            assert!(terminal.turn > state.turn || terminal.pending.is_some());
        }
    }

    /// #3386: the listener guard itself. A park inside a guarded listener
    /// refuses under that listener's name; an inner `None` (proven
    /// commutation) re-opens the carrier; leaving restores the outer name.
    #[test]
    fn deferred_choice_listener_guard_names_the_innermost_listener() {
        let fixture = stratagem_enemy_hit();
        let (state, _) = rooted(&fixture);
        assert_eq!(super::deferred_choice_wall(), None);
        for wall in [
            super::AFTER_SIDE_TURN_END_WALL,
            super::BEFORE_SIDE_TURN_END_WALL,
            super::BEFORE_SIDE_TURN_START_WALL,
            super::AFTER_DEATH_WALL,
            super::JOSS_SIDE_END_WALL,
            super::PAELS_EYE_JOSS_WALL,
            super::CONSTRICT_PUZZLE_WALL,
        ] {
            let guard = CarrierGuard::enter();
            {
                let _outer = super::DeferredChoiceListener::enter(Some(wall));
                assert_eq!(
                    super::park(&state),
                    Err(EngineRefusal::PowerOrderNotModeled(wall))
                );
                {
                    let _inner = super::DeferredChoiceListener::enter(None);
                    assert_eq!(
                        super::park(&state),
                        Err(EngineRefusal::MalformedArgs(PARK_SITE))
                    );
                }
                assert_eq!(super::deferred_choice_wall(), Some(wall));
            }
            assert_eq!(super::deferred_choice_wall(), None);
            let mut scratch = state.clone();
            // The one snapshot came from the inner (commuting) park.
            assert_eq!(
                guard.settle(&mut scratch, Err(EngineRefusal::PowerOrderNotModeled(wall))),
                Err(EngineRefusal::ContinuationNotModeled)
            );
        }
    }

    /// #3485: the guard's unprompted-select scope. Under a named wall a
    /// signalling note refuses when the listener settles, replacing any other
    /// result (a park unwinding from an inner commuting listener included); a
    /// non-signalling note (ending combat, a `Selector`) does not count; an
    /// inner `None` listener counts nothing and charges nothing outward; and
    /// leaving restores the outer count.
    #[test]
    fn deferred_choice_listener_refuses_a_select_resolved_without_a_prompt() {
        use crate::engine::hook_action::note_unprompted_select;
        let refused = Err::<(), _>(EngineRefusal::PowerOrderNotModeled(
            super::AUTO_RESOLVED_IN_DEFERRED_LISTENER,
        ));
        // Outside any listener a note is a no-op.
        note_unprompted_select(true);
        for wall in [
            super::AFTER_SIDE_TURN_END_WALL,
            super::BEFORE_SIDE_TURN_END_WALL,
            super::BEFORE_SIDE_TURN_START_WALL,
            super::AFTER_DEATH_WALL,
            super::JOSS_SIDE_END_WALL,
            super::PAELS_EYE_JOSS_WALL,
            super::CONSTRICT_PUZZLE_WALL,
        ] {
            // No note: the listener's own result passes through.
            let quiet = super::DeferredChoiceListener::enter(Some(wall));
            note_unprompted_select(false);
            assert_eq!(quiet.settle(Ok(7)), Ok(7));
            let quiet = super::DeferredChoiceListener::enter(Some(wall));
            assert_eq!(
                quiet.settle::<()>(Err(EngineRefusal::ContinuationNotModeled)),
                Err(EngineRefusal::ContinuationNotModeled)
            );

            // A signalling note refuses, whatever the listener returned.
            let noted = super::DeferredChoiceListener::enter(Some(wall));
            note_unprompted_select(true);
            assert_eq!(noted.settle(Ok(())), refused);
            let noted = super::DeferredChoiceListener::enter(Some(wall));
            note_unprompted_select(true);
            assert_eq!(
                noted.settle(Err(EngineRefusal::MalformedArgs(PARK_SITE))),
                refused
            );

            // A commuting inner listener absorbs its own notes; a named
            // inner listener refuses on its own and leaves the outer count
            // untouched; the outer still counts its own notes afterwards.
            let outer = super::DeferredChoiceListener::enter(Some(wall));
            {
                let inner = super::DeferredChoiceListener::enter(None);
                note_unprompted_select(true);
                assert_eq!(inner.settle(Ok(())), Ok(()));
            }
            {
                let inner = super::DeferredChoiceListener::enter(Some(wall));
                note_unprompted_select(true);
                assert_eq!(inner.settle(Ok(())), refused);
            }
            assert_eq!(super::deferred_choice_wall(), Some(wall));
            assert_eq!(outer.settle(Ok(())), Ok(()));
            let outer = super::DeferredChoiceListener::enter(Some(wall));
            {
                let _inner = super::DeferredChoiceListener::enter(None);
            }
            note_unprompted_select(true);
            assert_eq!(outer.settle(Ok(())), refused);
            assert_eq!(super::deferred_choice_wall(), None);
        }
        // A bare `None` listener never refuses.
        let exempt = super::DeferredChoiceListener::enter(None);
        note_unprompted_select(true);
        assert_eq!(exempt.settle(Ok(())), Ok(()));
    }

    /// #3485 witness (Joss Paper's side-end Draw, `relics.rs`): the same
    /// reshuffle as the #3386 witnesses, but against Stratagem 5, so
    /// `FromCombatPile` auto-takes all five Defends. Native still signals
    /// Joss Paper's listener context first and starts the later Parrying
    /// Shield before the take, so the inline take refuses by name. With no
    /// later listener (`None`) the same take completes in place.
    #[test]
    fn joss_paper_side_end_auto_take_refuses_before_a_later_listener() {
        let mut fixture = joss_side_end(&[RelicId::RelicParryingShield], false);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 5);
        let (state, catalog) = loaded(&fixture);
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::AUTO_RESOLVED_IN_DEFERRED_LISTENER)
        );

        let mut alone = joss_side_end(&[], false);
        alone.state.powers.set(PowerId::Stratagem, SlotWire::Int, 5);
        assert_eq!(
            super::super::relics::joss_paper_side_end_draw_wall(&alone.catalog, &alone.state),
            None
        );
        let (state, catalog) = loaded(&alone);
        let next = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(next.pending.is_none());
        assert_eq!(next.fanouts.joss_paper_cards_exhausted(), 0);
        assert!(next.turn > state.turn);
    }

    /// #3485 witness (Pael's Eye's Early listener, `turn.rs`): the #3386
    /// Pael's Eye fixture against Stratagem 5. Joss Paper's Draw inside the
    /// Exhaust auto-takes the five reshuffled Defends, which native defers
    /// behind every later BeforeSideTurnEnd listener, so it refuses by name.
    #[test]
    fn joss_paper_auto_take_under_paels_eye_refuses() {
        let identities = [plain(CardId::DefendIronclad)];
        let mut fixture = fixture_with(
            &identities,
            &[RelicId::RelicPaelsEye, RelicId::RelicJossPaper],
            2,
        );
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
        fixture
            .state
            .fanouts
            .set_paels_eye_was_owner_part_last_player_turn(true);
        let defend = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 5);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, defend));
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = loaded(&fixture);
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::AUTO_RESOLVED_IN_DEFERRED_LISTENER)
        );
    }

    // ---- History Course (#3309) ----

    /// A Toadpole fight holding History Course whose turn-1 Attack `retained`
    /// (already in Discard) is replayed as a dupe at turn 2's AutoPre.
    fn history_course(cards: &[CardId], extra_relics: &[RelicId]) -> Fixture {
        let identities = cards.iter().map(|id| plain(*id)).collect::<Vec<_>>();
        let mut relics = vec![RelicId::RelicHistoryCourse];
        relics.extend_from_slice(extra_relics);
        let mut fixture = fixture_with(&identities, &relics, 2);
        fixture.state.turn = 1;
        fixture
    }

    fn retain(fixture: &mut Fixture, uid: u32, atom: CardAtom) {
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(card(uid, atom));
        fixture
            .state
            .fanouts
            .set_history_course_attack_current_turn(Some(crate::hot::FrozenAutoBatchEntry {
                card: card(uid, atom),
                state: crate::hot::CardInstanceState::default(),
            }));
    }

    /// The parked History Course phase's dupe uid and the frames above it.
    fn parked_history_course(parked: &HotState) -> (u32, Vec<crate::frame::Frame>) {
        assert!(parked.pending.is_some());
        assert!(matches!(
            parked.frames.as_slice().first(),
            Some(crate::frame::Frame::ActionReplay { .. })
        ));
        let depth = parked_draw_depth(parked).expect("a receipt-owned frame");
        let crate::frame::Frame::Phase { record } = parked.frames.as_slice()[depth] else {
            panic!("{:#?}", parked.frames);
        };
        let dupe = parked.frames.auto_pre_history_course_phase(record).unwrap();
        (dupe, parked.frames.as_slice()[depth + 1..].to_vec())
    }

    /// #3309 witness, selecting child: History Course replays Photon Cut
    /// (Attack 10, Draw 1, put one Hand card on top of the Draw pile). The
    /// dupe parks on its own Hand selection inside EndTurn; every answer
    /// finishes the dupe once, moves exactly the chosen card, and finishes
    /// AutoPre once.
    #[test]
    fn history_course_photon_cut_dupe_parks_on_its_selection_and_finishes_once() {
        let mut fixture = history_course(&[CardId::PhotonCut, CardId::DefendIronclad], &[]);
        let (photon, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..10).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        super::take_branch_hits();
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let (dupe, above) = parked_history_course(&parked);
        assert_eq!(above.len(), 1, "{above:#?}");
        assert_eq!(parked.turn, 2);
        assert_eq!(parked.player_phase, crate::engine::turn::PHASE_AUTO_PRE);
        assert!(
            parked
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .any(|c| c.uid == dupe)
        );
        // The Hand Draw (5) and the dupe's Draw 1.
        assert_eq!(parked.piles.get(PileId::Hand).len(), 6);
        let hp_after_hit = parked.monsters[0].hp;
        assert_eq!(hp_after_hit, 1_000 - 10);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert_eq!(terminals.len(), 6);
        let hits = super::take_branch_hits();
        assert!(hits.history_course_answers >= 6, "{hits:?}");
        let mut tops = std::collections::BTreeSet::new();
        for terminal in &terminals {
            assert_eq!(terminal.turn, 2);
            assert_eq!(terminal.player_phase, PHASE_ORDINARY_ACTIONS);
            assert_eq!(terminal.monsters[0].hp, hp_after_hit);
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 5);
            assert!(
                PileId::ALL
                    .into_iter()
                    .flat_map(|pile| terminal.piles.get(pile).as_slice())
                    .all(|c| c.uid != dupe)
            );
            let top = terminal.piles.get(PileId::Draw).as_slice()[0].uid;
            assert!(
                terminal
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .all(|c| c.uid != top)
            );
            tops.insert(top);
        }
        assert_eq!(tops.len(), 6, "each answer moves its own card");
    }

    /// Pommel Strike (Attack 9, Draw 1) retained under Hellraiser 1, with
    /// the Hand Draw's five Defends above `after` in the Draw pile.
    fn history_course_hellraiser(after: &[CardId]) -> Fixture {
        let mut cards = vec![CardId::PommelStrike, CardId::DefendIronclad];
        cards.extend_from_slice(after);
        let mut fixture = history_course(&cards, &[]);
        let (pommel, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        retain(&mut fixture, 1, pommel);
        let atoms = fixture.atoms[2..].to_vec();
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.extend((2..7).map(|uid| card(uid, defend)));
        for (offset, atom) in (7..).zip(atoms) {
            draw.push(card(offset, atom));
        }
        draw.extend((12..16).map(|uid| card(uid, defend)));
        fixture
    }

    /// #3309 witness, Hellraiser: the dupe's Draw 1 reaches Seeker Strike,
    /// Hellraiser AutoPlays it and it parks on its Draw-pile choice. The
    /// parked stack is the phase, the dupe, the dupe's CardPlay Draw and the
    /// Hellraiser child; every answer finishes both plays once.
    #[test]
    fn history_course_dupe_draw_reaching_a_selecting_hellraiser_strike_parks_and_finishes_once() {
        let fixture = history_course_hellraiser(&[CardId::SeekerStrike]);
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let (dupe, above) = parked_history_course(&parked);
        assert!(
            matches!(
                above.as_slice(),
                [
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "{above:#?}"
        );
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(terminals.len() > 1);
        for terminal in &terminals {
            assert_eq!(terminal.turn, 2);
            assert_eq!(terminal.player_phase, PHASE_ORDINARY_ACTIONS);
            // The dupe and the Hellraiser Seeker each finish once.
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
            assert_eq!(terminal.monsters[0].hp, terminals[0].monsters[0].hp);
            assert!(terminal.monsters[0].hp <= 1_000 - 9 - 9);
            assert!(
                PileId::ALL
                    .into_iter()
                    .flat_map(|pile| terminal.piles.get(pile).as_slice())
                    .all(|c| c.uid != dupe)
            );
        }
    }

    /// #3309 witness, two parks in one dupe: Photon Cut's Draw 1 reaches a
    /// selecting Hellraiser Seeker Strike (first park); after that answer the
    /// dupe itself parks on its Hand selection (second park). The second
    /// answer re-executes the receipt with a non-last tape answer consumed
    /// in place.
    #[test]
    fn history_course_photon_cut_dupe_parks_twice_and_consumes_a_non_last_answer_in_place() {
        let mut fixture = history_course(
            &[
                CardId::PhotonCut,
                CardId::DefendIronclad,
                CardId::SeekerStrike,
            ],
            &[],
        );
        let (photon, defend, seeker) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        fixture
            .state
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
        retain(&mut fixture, 1, photon);
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.extend((2..7).map(|uid| card(uid, defend)));
        draw.push(card(7, seeker));
        draw.extend((8..12).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let first = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let (_, above) = parked_history_course(&first);
        assert_eq!(above.len(), 3, "{above:#?}");
        super::take_branch_hits();
        let (terminals, visited) = every_answer_to_terminal(&first, &catalog);
        assert!(
            visited > 1,
            "the dupe's own selection parks after the Seeker"
        );
        let hits = super::take_branch_hits();
        assert!(hits.inline_non_last > 0, "{hits:?}");
        assert!(hits.history_course_answers > 0, "{hits:?}");
        for terminal in &terminals {
            assert_eq!(terminal.turn, 2);
            assert_eq!(terminal.player_phase, PHASE_ORDINARY_ACTIONS);
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
        }
    }

    /// #3309 witness, Stratagem: the dupe's Draw finds an empty Draw pile and
    /// its reshuffle parks on Stratagem 2. The answer resumes the Draw frame
    /// inside the dupe, whose remaining frames are then driven back to the
    /// History Course phase (`play::drive_receipt_owned_history_course_inline`).
    #[test]
    fn history_course_dupe_draw_parks_on_stratagem_and_drives_back_to_the_phase() {
        let mut fixture = history_course(&[CardId::PommelStrike, CardId::DefendIronclad], &[]);
        let (pommel, defend) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 2);
        retain(&mut fixture, 1, pommel);
        fixture
            .state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((10..14).map(|uid| card(uid, defend)));
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..7).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        super::take_branch_hits();
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let (_, above) = parked_history_course(&parked);
        assert!(
            matches!(
                above.as_slice(),
                [
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. }
                ]
            ),
            "{above:#?}"
        );
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(terminals.len() > 1);
        assert!(super::take_branch_hits().history_course_drives > 0);
        for terminal in &terminals {
            assert_eq!(terminal.turn, 2);
            assert_eq!(terminal.player_phase, PHASE_ORDINARY_ACTIONS);
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
        }
    }

    /// #3309 witness, the receipt-owned phase completing without a park: the
    /// dupe's Draw reaches a Defend, so the phase is pushed and popped inside
    /// the one EndTurn and nothing is published.
    #[test]
    fn history_course_suspending_capable_dupe_that_does_not_park_completes_in_place() {
        let fixture = history_course_hellraiser(&[CardId::DefendIronclad, CardId::SeekerStrike]);
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let next = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(next.pending.is_none());
        assert!(next.frames.is_empty());
        assert_eq!(next.turn, 2);
        assert_eq!(next.history.card_plays_finished_combat, 1);
        assert_eq!(next.monsters[0].hp, 1_000 - 9);
        cold(&next, &catalog);
    }

    /// #3309 witness, `turn::history_course_dupe_is_receipt_owned` false: a
    /// plain Strike keeps the synchronous path even where Photon Cut makes
    /// the fight replay-capable.
    #[test]
    fn history_course_non_suspending_dupe_keeps_the_synchronous_path() {
        let mut fixture = history_course(
            &[
                CardId::PhotonCut,
                CardId::DefendIronclad,
                CardId::StrikeIronclad,
            ],
            &[],
        );
        let (photon, defend, strike) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        retain(&mut fixture, 1, strike);
        let draw = fixture.state.piles.get_mut(PileId::Draw).make_mut();
        draw.extend((2..10).map(|uid| card(uid, defend)));
        // A live Photon Cut is what makes this fight replay-capable.
        draw.push(card(10, photon));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let next = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(next.pending.is_none() && next.frames.is_empty());
        assert_eq!(next.history.card_plays_finished_combat, 1);
        assert_eq!(next.monsters[0].hp, 1_000 - 6);
    }

    /// #3309 witness, the two named refusals of
    /// `turn::history_course_dupe_is_receipt_owned`: a retained Uproar (an
    /// Attack that spawns a further AutoPlay) and a suspending dupe reached
    /// outside an ActionReplay transaction.
    #[test]
    fn history_course_refuses_a_batch_parent_dupe_and_an_unrooted_suspending_dupe_by_name() {
        let mut fixture = history_course(&[CardId::Uproar, CardId::DefendIronclad], &[]);
        let (uproar, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, uproar);
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..10).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert_eq!(
            apply_action(&state, &catalog, &Action::EndTurn).map(|_| ()),
            Err(EngineRefusal::MalformedArgs(
                "History Course dupe AutoPlay batch parent"
            ))
        );

        let mut fixture = history_course(&[CardId::PhotonCut, CardId::DefendIronclad], &[]);
        let (photon, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((2..5).map(|uid| card(uid, defend)));
        let (mut state, catalog) = rooted(&fixture);
        state.turn = 2;
        state.fanouts.roll_history_course_attack_turn();
        assert_eq!(
            crate::engine::turn::finish_auto_pre_relic_tail(&mut state, &catalog, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "History Course suspending dupe outside an ActionReplay transaction"
            ))
        );
    }

    /// #3309 witness, the canonical phase: it projects its precommitted
    /// listener and the dupe's uid, and a phase whose local is malformed or
    /// names a uid other than the CardPlay above it does not load.
    #[test]
    fn history_course_parked_phase_forgeries_do_not_load() {
        let mut fixture = history_course(&[CardId::PhotonCut, CardId::DefendIronclad], &[]);
        let (photon, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..10).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let (dupe, _) = parked_history_course(&parked);
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let phase_index = wire
            .continuations
            .iter()
            .position(|frame| frame.fields.get("phase") == Some(&serde_json::json!("auto_pre")))
            .unwrap();
        let fields = &wire.continuations[phase_index].fields;
        assert_eq!(
            fields.get("snapshot"),
            Some(&serde_json::json!([["relic", "HISTORY_COURSE"]]))
        );
        assert_eq!(fields.get("locals"), Some(&serde_json::json!([dupe])));
        for locals in [
            serde_json::json!([]),
            serde_json::json!([0]),
            serde_json::json!([dupe, dupe]),
            serde_json::json!(["x"]),
            serde_json::json!([dupe - 1]),
        ] {
            let mut forged = wire.clone();
            forged.continuations[phase_index]
                .fields
                .insert("locals".to_owned(), locals.clone());
            assert!(
                HotBoundary::from_canonical(&forged, &loaded_catalog).is_err(),
                "{locals}"
            );
        }
        for (field, value) in [
            ("subphase", serde_json::json!("late")),
            ("cursor", serde_json::json!(0)),
            ("remaining", serde_json::json!([])),
        ] {
            let mut forged = wire.clone();
            forged.continuations[phase_index]
                .fields
                .insert(field.to_owned(), value);
            assert!(HotBoundary::from_canonical(&forged, &loaded_catalog).is_err());
        }
    }

    /// A parked Photon Cut dupe (see the first History Course witness).
    fn parked_photon_cut() -> (HotState, Catalog) {
        let mut fixture = history_course(&[CardId::PhotonCut, CardId::DefendIronclad], &[]);
        let (photon, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..10).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        (parked, catalog)
    }

    /// Rebuild `parked`'s `[receipt, phase, dupe]` stack with the given root
    /// action and phase uid.
    fn rebuild_history_course_stack(
        parked: &HotState,
        action: crate::hot::ActionReplayRootAction,
        phase_uid: u32,
    ) -> HotState {
        let frames = parked.frames.as_slice();
        let [
            crate::frame::Frame::ActionReplay { record: root },
            crate::frame::Frame::Phase { .. },
            crate::frame::Frame::CardPlay { record: play },
        ] = frames
        else {
            panic!("{frames:#?}");
        };
        let mut replay = parked.frames.action_replay(*root).unwrap();
        replay.action = action;
        let play = parked.frames.card_play(*play).unwrap().to_owned();
        let mut rebuilt = parked.clone();
        rebuilt.frames = crate::hot::Frames::new();
        rebuilt.frames.push_action_replay(&replay).unwrap();
        rebuilt
            .frames
            .push_auto_pre_history_course_phase(phase_uid)
            .unwrap();
        let record = rebuilt.frames.push_card_play(&play).unwrap();
        rebuilt.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
            frame_uid: play.uid,
            frame_record: record,
        }));
        rebuilt
    }

    /// #3309 witness, `play::receipt_owned_history_course_stack_is_exact`:
    /// the rebuilt stack validates, and each forged binding is refused — a
    /// phase naming another uid, a receipt that is not an EndTurn, a turn-1
    /// or non-AutoPre state.
    #[test]
    fn history_course_parked_stack_grammar_refuses_each_forged_binding() {
        use crate::engine::play::persisted_card_play_stack_is_exact;
        use crate::hot::ActionReplayRootAction;

        let (parked, catalog) = parked_photon_cut();
        let (dupe, _) = parked_history_course(&parked);
        let exact = rebuild_history_course_stack(&parked, ActionReplayRootAction::EndTurn, dupe);
        assert_eq!(persisted_card_play_stack_is_exact(&exact, &catalog), Ok(()));

        let other_uid =
            rebuild_history_course_stack(&parked, ActionReplayRootAction::EndTurn, dupe - 1);
        assert!(persisted_card_play_stack_is_exact(&other_uid, &catalog).is_err());
        let play_root = rebuild_history_course_stack(
            &parked,
            ActionReplayRootAction::Play {
                uid: 2,
                target: None,
                selection_uid: None,
            },
            dupe,
        );
        assert!(persisted_card_play_stack_is_exact(&play_root, &catalog).is_err());
        let mut first_turn = exact.clone();
        first_turn.turn = 1;
        assert!(persisted_card_play_stack_is_exact(&first_turn, &catalog).is_err());
        let mut ordinary = exact.clone();
        ordinary.player_phase = PHASE_ORDINARY_ACTIONS;
        assert!(persisted_card_play_stack_is_exact(&ordinary, &catalog).is_err());
    }

    /// #3309 witness, the nested-AutoPlay guard in `play_card_with_work`: a
    /// History Course phase admits exactly the dupe it names as its child.
    #[test]
    fn history_course_phase_admits_only_the_dupe_it_names() {
        let mut fixture = history_course(&[CardId::PhotonCut, CardId::DefendIronclad], &[]);
        let (photon, defend) = (fixture.atoms[0], fixture.atoms[1]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((2..5).map(|uid| card(uid, defend)));
        let (mut state, catalog) = rooted(&fixture);
        state.turn = 2;
        state.player_phase = crate::engine::turn::PHASE_AUTO_PRE;
        state.fanouts.roll_history_course_attack_turn();
        let source = state
            .fanouts
            .history_course_attack_previous_turn()
            .cloned()
            .unwrap();
        let dupe = state.next_card_uid;
        let mut wrong = state.clone();
        wrong
            .frames
            .push_auto_pre_history_course_phase(dupe + 1)
            .unwrap();
        assert_eq!(
            crate::engine::play::autoplay_history_course_dupe(
                &mut wrong,
                &catalog,
                source.clone(),
                &mut Vec::new()
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        let mut named = state.clone();
        named
            .frames
            .push_auto_pre_history_course_phase(dupe)
            .unwrap();
        crate::engine::play::autoplay_history_course_dupe(
            &mut named,
            &catalog,
            source,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(named.pending.is_some());
    }

    /// #3309 witness, `engine::service_void_form_end_turn` on the replay
    /// path: Void Form's requested EndTurn is its own public EndTurn
    /// transaction, so a History Course dupe that parks in the new turn
    /// carries an EndTurn receipt whose predecessor is the completed play.
    #[test]
    fn void_form_requested_end_turn_parks_a_history_course_dupe_on_an_end_turn_receipt() {
        let mut fixture = history_course(
            &[CardId::PhotonCut, CardId::DefendIronclad, CardId::VoidForm],
            &[],
        );
        let (photon, defend, void_form) = (fixture.atoms[0], fixture.atoms[1], fixture.atoms[2]);
        retain(&mut fixture, 1, photon);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(15, void_form));
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((2..10).map(|uid| card(uid, defend)));
        let (state, catalog) = rooted(&fixture);
        assert!(catalog.requires_action_replay());
        let play = Action::Play {
            uid: 15,
            target: None,
            selection: SelectionRef::NONE,
        };
        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        let (_, above) = parked_history_course(&parked);
        assert_eq!(above.len(), 1);
        assert_eq!(parked.turn, 2);
        let crate::frame::Frame::ActionReplay { record } = parked.frames.as_slice()[0] else {
            unreachable!()
        };
        let replay = parked.frames.action_replay(record).unwrap();
        assert_eq!(replay.action, crate::hot::ActionReplayRootAction::EndTurn);
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        for terminal in &terminals {
            assert_eq!(terminal.turn, 2);
            assert_eq!(terminal.player_phase, PHASE_ORDINARY_ACTIONS);
            // Void Form, then the dupe.
            assert_eq!(terminal.history.card_plays_finished_combat, 2);
        }
    }

    // ---- #3400: AfterSideTurnEnd relic listeners in inventory order --------

    /// A Joss Paper side end with no park: Joss Paper at 4, one Dazed in Hand
    /// (its side-end Ethereal exhaust makes the count 5), and `top` alone in
    /// the Draw pile for the threshold Draw. No Stratagem, so nothing selects.
    /// The Toadpole has `monster_hp`, and `relics` hold the inventory in that
    /// order, vouched or not.
    fn joss_order(relics: &[RelicId], vouched: bool, monster_hp: i32, top: CardId) -> Fixture {
        let identities = [plain(CardId::Dazed), plain(top)];
        let mut fixture = fixture_in_order(&identities, relics, 2, vouched);
        assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
        fixture.state.monsters_mut()[0].hp = monster_hp;
        let (dazed, top) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(card(1, dazed));
        fixture
            .state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(2, top));
        fixture
    }

    fn end_turn(fixture: &Fixture) -> Result<HotState, EngineRefusal> {
        let (state, catalog) = loaded(fixture);
        apply_action(&state, &catalog, &Action::EndTurn).map(|next| next.state)
    }

    /// #3400 witness: Joss Paper's side-end Draw and a lethal Lunar Pastry or
    /// Parrying Shield hit, in both vouched orders.
    ///
    /// Lunar Pastry's Star fires Black Hole (6) and Parrying Shield (Block
    /// 10) deals 6, each killing the 6-HP Toadpole. Native runs the relic
    /// listeners in `Player.Relics` order, and Joss Paper's Draw is a no-op
    /// once the fight is ending. So the peer recorded first kills before the
    /// Draw and the Hand stays empty, while Joss Paper recorded first draws
    /// its card and the peer kills after. Joss Paper's counter settles at 0
    /// either way.
    #[test]
    fn joss_paper_draw_follows_its_recorded_side_of_a_lethal_peer() {
        for peer in [RelicId::RelicLunarPastry, RelicId::RelicParryingShield] {
            for joss_first in [true, false] {
                let relics = if joss_first {
                    [RelicId::RelicJossPaper, peer]
                } else {
                    [peer, RelicId::RelicJossPaper]
                };
                let mut fixture = joss_order(&relics, true, 6, CardId::DefendIronclad);
                fixture.state.block = 10;
                fixture
                    .state
                    .powers
                    .set(PowerId::BlackHole, SlotWire::Int, 6);
                crate::engine::play::hydrate_after_card_played_power_order_for_test(
                    &mut fixture.state,
                );
                let terminal =
                    end_turn(&fixture).unwrap_or_else(|refusal| panic!("{relics:?}: {refusal:?}"));
                assert!(terminal.history.over, "{relics:?}");
                assert_eq!(terminal.monsters[0].hp, 0, "{relics:?}");
                assert_eq!(
                    terminal.piles.get(PileId::Hand).len(),
                    usize::from(joss_first),
                    "{relics:?}: the Draw ran only ahead of the kill"
                );
                assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
                // Lunar Pastry's Star lands in both orders: the kill is its own.
                assert_eq!(
                    terminal.stars,
                    i16::from(peer == RelicId::RelicLunarPastry),
                    "{relics:?}"
                );
            }
        }
    }

    /// #3400 witness: under a live Hellraiser, Joss Paper's Draw AutoPlays
    /// the Strike it draws, and Art of War's and Kusarigama's rollovers
    /// count it on their recorded side.
    ///
    /// The Strike kills the 6-HP Toadpole, so the fight ends at this side end
    /// and the rollovers are what the terminal state holds. Art of War or
    /// Kusarigama recorded before Joss Paper rolls over first and then sees
    /// the Attack (`AnyAttacksPlayedThisTurn` set, Kusarigama at 1). Joss
    /// Paper recorded first plays it before the rollover, which moves the
    /// flag into `AnyAttacksPlayedLastTurn` and zeroes Kusarigama.
    #[test]
    fn hellraiser_joss_draw_is_counted_on_the_recorded_side_of_the_rollovers() {
        for joss_first in [true, false] {
            let relics = if joss_first {
                [
                    RelicId::RelicJossPaper,
                    RelicId::RelicArtOfWar,
                    RelicId::RelicKusarigama,
                ]
            } else {
                [
                    RelicId::RelicArtOfWar,
                    RelicId::RelicKusarigama,
                    RelicId::RelicJossPaper,
                ]
            };
            let mut fixture = joss_order(&relics, true, 6, CardId::StrikeIronclad);
            fixture
                .state
                .powers
                .set(PowerId::Hellraiser, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(
                &mut fixture.state,
            );
            let terminal =
                end_turn(&fixture).unwrap_or_else(|refusal| panic!("{relics:?}: {refusal:?}"));
            assert!(terminal.history.over, "{relics:?}: the Strike killed");
            assert_eq!(terminal.history.card_plays_finished_combat, 1);
            assert_eq!(terminal.art_of_war_current_attack(), !joss_first);
            assert_eq!(terminal.art_of_war_last_attack(), joss_first);
            assert_eq!(terminal.fanouts.kusarigama(), u8::from(!joss_first));
        }
    }

    /// #3400 witness: an unvouched inventory keeps the fixed order and
    /// refuses by name exactly where it is not proven native: a threshold
    /// Draw beside Lunar Pastry or Parrying Shield, or beside Art of War or
    /// Kusarigama under a live Hellraiser. The same inventories vouched run
    /// (the witnesses above), and so do the commuting unvouched rows: no
    /// Draw, and Art of War or Kusarigama with no card playable.
    #[test]
    fn unvouched_side_end_relic_pairs_refuse_only_where_the_order_shows() {
        let order = EngineRefusal::PowerOrderNotModeled(
            crate::engine::relics::AFTER_SIDE_TURN_END_RELIC_ORDER,
        );
        let joss = RelicId::RelicJossPaper;
        for (peer, hellraiser, refuses) in [
            (RelicId::RelicLunarPastry, false, true),
            (RelicId::RelicParryingShield, false, true),
            (RelicId::RelicArtOfWar, true, true),
            (RelicId::RelicKusarigama, true, true),
            (RelicId::RelicArtOfWar, false, false),
            (RelicId::RelicKusarigama, false, false),
        ] {
            for relics in [[joss, peer], [peer, joss]] {
                for vouched in [false, true] {
                    let mut fixture = joss_order(&relics, vouched, 1_000, CardId::StrikeIronclad);
                    fixture.state.block = 10;
                    if hellraiser {
                        fixture
                            .state
                            .powers
                            .set(PowerId::Hellraiser, SlotWire::Int, 1);
                        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(
                            &mut fixture.state,
                        );
                    }
                    let result = end_turn(&fixture);
                    if refuses && !vouched {
                        assert_eq!(result.err(), Some(order.clone()), "{relics:?}");
                    } else {
                        result.unwrap_or_else(|refusal| {
                            panic!("{relics:?} vouched={vouched}: {refusal:?}")
                        });
                    }
                    // Below the threshold there is no Draw, and no order.
                    assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(0));
                    end_turn(&fixture).unwrap_or_else(|refusal| {
                        panic!("{relics:?} vouched={vouched}, no Draw: {refusal:?}")
                    });
                }
            }
        }
    }

    /// #3400 witness, the narrowed #3386 wall: on a vouched inventory a peer
    /// recorded BEFORE Joss Paper has already run when its Draw parks, so it
    /// is not a later listener. Lunar Pastry, Parrying Shield (and, under
    /// Hellraiser, Art of War and Kusarigama) recorded first leave the Draw
    /// free to park; recorded after, or on an unvouched inventory, they
    /// still wall it, at admission and at runtime.
    #[test]
    fn a_peer_recorded_before_joss_paper_no_longer_walls_its_draw() {
        let joss = RelicId::RelicJossPaper;
        let wall = Some(super::JOSS_SIDE_END_WALL);
        for (peer, hellraiser) in [
            (RelicId::RelicLunarPastry, false),
            (RelicId::RelicParryingShield, false),
            (RelicId::RelicArtOfWar, true),
            (RelicId::RelicKusarigama, true),
        ] {
            for (relics, vouched, walled) in [
                ([peer, joss], true, false),
                ([joss, peer], true, true),
                ([peer, joss], false, true),
                ([joss, peer], false, true),
            ] {
                let mut fixture =
                    fixture_in_order(&[plain(CardId::DefendIronclad)], &relics, 0, vouched);
                if hellraiser {
                    fixture
                        .state
                        .powers
                        .set(PowerId::Hellraiser, SlotWire::Int, 1);
                }
                assert_eq!(
                    super::super::relics::joss_paper_side_end_draw_wall(
                        &fixture.catalog,
                        &fixture.state
                    ),
                    if walled { wall } else { None },
                    "{relics:?} vouched={vouched}"
                );
            }
        }
    }

    /// #3400 witness, end to end: Parrying Shield recorded before Joss Paper
    /// (vouched) acts first, and Joss Paper's side-end Draw then parks on the
    /// Stratagem reshuffle (the #3386 fixture). The root admits, every
    /// answer resolves, and Parrying Shield's hit lands exactly once. The
    /// reverse order still refuses at admission and at runtime.
    #[test]
    fn joss_paper_draw_parks_after_a_parrying_shield_recorded_first() {
        let order = |relics: &[RelicId]| {
            let identities = [plain(CardId::Dazed), plain(CardId::DefendIronclad)];
            let mut fixture = fixture_in_order(&identities, relics, 2, true);
            assert!(fixture.state.fanouts.set_joss_paper_cards_exhausted(4));
            let (dazed, defend) = (fixture.atoms[0], fixture.atoms[1]);
            fixture
                .state
                .powers
                .set(PowerId::Stratagem, SlotWire::Int, 2);
            fixture.state.block = 10;
            let hand = fixture.state.piles.get_mut(PileId::Hand).make_mut();
            hand.push(card(1, dazed));
            hand.extend((2..7).map(|uid| card(uid, defend)));
            fixture
        };
        let first = order(&[RelicId::RelicParryingShield, RelicId::RelicJossPaper]);
        let (state, catalog) = rooted(&first);
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(parked_joss_draw(&parked), (1, 0));
        assert_eq!(
            parked.monsters[0].hp, 994,
            "Parrying Shield hit before the Draw"
        );
        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(visited, 1);
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            assert_eq!(terminal.fanouts.joss_paper_cards_exhausted(), 0);
            assert_eq!(terminal.monsters[0].hp, parked.monsters[0].hp);
        }

        let after = order(&[RelicId::RelicJossPaper, RelicId::RelicParryingShield]);
        let (state, catalog) = loaded(&after);
        assert!(refuses_at_admission(
            &state,
            &catalog,
            super::JOSS_SIDE_END_WALL
        ));
        assert_eq!(
            end_turn_refusal(&state, &catalog),
            EngineRefusal::PowerOrderNotModeled(super::JOSS_SIDE_END_WALL)
        );
    }
}
