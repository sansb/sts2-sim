"""#3936: three checkpoint-pairing rules of the census's `NativeChecks`.

Each is a capture shape the game records and the census used to report as an
engine disagreement:

* a hook action that resumes with no choice of the player's inside it (the
  turn-start AutoPrePlay dispatch, `CombatManager/<StartTurn>d__100` RVA
  0x3f781c IL_0b1e-IL_0b29; site fights DA0H5Y4A7UF8 n21 and S09VM8AM9XU6
  n16) owns its checkpoint, compared against the state the next input sees;
* an AutoPost hook whose HookAction came BEFORE the choice a listener of it
  opened (Stampede; SH3H787N0453 n43) is compared at the dispatch's own
  staged state by the select that answers it;
* a play the game itself cancelled (`PlayCardAction/<ExecuteAction>d__25`
  RVA 0x3d5458: `CanPlay` IL_018d, `GameAction::Cancel` IL_01a8; QF89PUW7MNBC
  n48) is skipped only when the capture's own checkpoint for it is the
  engine's state before the play.
"""
import pathlib
import sys

import pytest

import rust_replay as replay

RUST_DIR = pathlib.Path(__file__).resolve().parents[1] / "engine"
sys.path.insert(0, str(RUST_DIR / "tools"))

import eval_suite  # noqa: E402

PLAY0 = 'finished action execution PlayCardAction card: X index: 0 targetid: '
PLAY1 = 'finished action execution PlayCardAction card: Y index: 1 targetid: '
HOOK0 = 'finished action execution GenericHookGameAction id 0 owner 1 source  last involved '
PLAY = {'action': {'type': 'NetPlayCardAction', 'combat_card_index': 0}}
HOOK = {'event_type': 'HookAction', 'hook_id': 0}
RESUME = {'event_type': 'ResumeAction', 'action_id': 7}
READY = {'event_type': 'GameAction', 'action': {'type': 'NetReadyToBeginEnemyTurnAction'}}
STAGED = {'player': {'turn': 1, 'marker': 'staged'}}
BEFORE = {'player': {'turn': 2, 'marker': 'before'}}
AFTER = {'player': {'turn': 2, 'marker': 'after'}}
PENDING = {'player': {'turn': 2, 'pending': ['frame_select']}}


@pytest.fixture
def compared(monkeypatch):
    seen = []
    monkeypatch.setattr(replay, 'check_native_snapshot', lambda state, full: seen.append(state))
    return seen


def _checks(*contexts):
    return replay.NativeChecks({'checksums': [
        {'context': context, 'full_state': {'n': index}} for index, context in enumerate(contexts)]})


def _applied(*entries):
    return {'digest': 'd', 'native_checkpoints': list(entries)}


# ---- a hook action that resumes with no choice ---------------------------


def test_a_hook_that_resumes_with_no_choice_owns_its_checkpoint(compared):
    checks = _checks(HOOK0, PLAY0)
    checks.stage(_applied())          # the EndTurn apply staged nothing
    checks.no_decision(READY, BEFORE)
    checks.start(HOOK)
    checks.no_decision(HOOK, BEFORE)
    assert compared == [] and checks.owner is not None
    checks.no_decision(RESUME, BEFORE)
    assert compared == [BEFORE] and checks.owner is None and checks.cursor == 1
    # The next card then finds its own checkpoint, not the hook's.
    checks.start(PLAY)
    checks.completed(AFTER)
    checks.finish()
    assert compared == [BEFORE, AFTER] and checks.validated == 2


def test_without_the_rule_the_next_play_would_meet_the_hooks_checkpoint(compared):
    # The pre-#3936 behaviour, reproduced by withholding the state: the hook
    # keeps its owner through the resume and the play overwrites it.
    checks = _checks(HOOK0, PLAY0)
    checks.start(HOOK)
    checks.no_decision(HOOK)
    checks.no_decision(RESUME)
    assert compared == [] and checks.owner is not None
    checks.start(PLAY)
    with pytest.raises(ValueError, match='action identity mismatch'):
        checks.completed(AFTER)


def test_a_resume_after_the_hooks_choice_was_answered_compares_nothing(compared):
    checks = _checks(HOOK0)
    checks.start(HOOK)
    checks.no_decision(HOOK, BEFORE)
    checks.completed(AFTER)           # the PlayerChoice's select wire
    checks.no_decision(RESUME, AFTER)
    checks.finish()
    assert compared == [AFTER]


def test_a_resume_while_the_hooks_choice_is_open_keeps_the_owner(compared):
    checks = _checks(HOOK0)
    checks.start(HOOK)
    checks.no_decision(HOOK, PENDING)
    checks.no_decision(RESUME, PENDING)
    assert compared == [] and checks.owner is not None and checks.cursor == 0


def test_a_resume_inside_a_card_play_is_not_a_hook_completion(compared):
    # A card whose own choice paused it: the PlayCardAction owns the next
    # checkpoint, and its resume is not a completed hook.
    checks = _checks(PLAY0)
    checks.start(PLAY)
    checks.no_decision(RESUME, BEFORE)
    assert compared == [] and checks.owner is not None


def test_a_hook_resume_that_disagrees_is_a_mismatch(monkeypatch):
    def differs(_state, _full):
        raise ValueError('recorded replay differs from native monsters')
    monkeypatch.setattr(replay, 'check_native_snapshot', differs)
    checks = _checks(HOOK0)
    checks.start(HOOK)
    checks.no_decision(HOOK, BEFORE)
    with pytest.raises(ValueError, match='differs from native monsters'):
        checks.no_decision(RESUME, BEFORE)


# ---- an AutoPost hook answered by a select -------------------------------


def test_a_select_that_answers_an_open_auto_post_hook_is_compared_at_the_staged_state(compared):
    checks = _checks(HOOK0)
    checks.stage(_applied())          # EndTurn parked on the listener's choice
    checks.start(HOOK)
    checks.no_decision(HOOK, PENDING)
    played = checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED}))
    assert played is STAGED and checks.auto_post is None
    checks.completed(played if played is not None else AFTER)
    checks.finish()
    assert compared == [STAGED]


def test_an_auto_post_state_with_no_open_hook_still_waits_for_its_hook_action(compared):
    # #3242's order is unchanged: the EndTurn apply stages, the HookAction
    # that follows claims it.
    checks = _checks(HOOK0)
    assert checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED})) is None
    assert checks.auto_post is STAGED
    checks.start(HOOK)
    checks.no_decision(HOOK, AFTER)
    assert compared == [STAGED]


def test_a_card_play_owner_does_not_claim_an_auto_post_state(compared):
    checks = _checks(PLAY0)
    checks.start(PLAY)
    assert checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED})) is None
    assert checks.auto_post is STAGED


# ---- a play the game cancelled -------------------------------------------


class _Recorded:
    """The slice of `NativeChecks` `_play_cancelled_by_the_game` reads."""

    def __init__(self, contexts, owner, cursor=0, full=True):
        self.checks = [{'context': context, 'full_state': {'n': index} if full else None}
                       for index, context in enumerate(contexts)]
        self.cursor, self.owner, self.done = cursor, owner, []

    def completed(self, state):
        self.done.append(state)
        self.cursor += 1
        self.owner = None


OWNER0 = ('PlayCardAction card:', ' index: 0 targetid:')


@pytest.fixture
def agrees(monkeypatch):
    monkeypatch.setattr(eval_suite, 'check_native_checkpoint', lambda state, full: None)


def test_a_play_whose_own_checkpoint_is_the_state_before_it_was_cancelled(agrees):
    checks = _Recorded([PLAY0, PLAY1], OWNER0)
    assert eval_suite._play_cancelled_by_the_game(checks, BEFORE)
    # Consumed as a validated checkpoint, against the state before the play.
    assert checks.done == [BEFORE] and checks.cursor == 1 and checks.owner is None


def test_a_play_whose_checkpoint_differs_is_not_cancelled(monkeypatch):
    def differs(_state, _full):
        raise ValueError('recorded replay differs from native player state')
    monkeypatch.setattr(eval_suite, 'check_native_checkpoint', differs)
    checks = _Recorded([PLAY0], OWNER0)
    assert not eval_suite._play_cancelled_by_the_game(checks, BEFORE)
    assert checks.done == [] and checks.cursor == 0 and checks.owner == OWNER0


@pytest.mark.parametrize('checks, state', [
    (None, BEFORE),                                        # a capture without checksums
    (_Recorded([PLAY0], None), BEFORE),                    # no action owns the next checkpoint
    (_Recorded([PLAY0], OWNER0, cursor=1), BEFORE),        # no checkpoint left
    (_Recorded([PLAY1], OWNER0), BEFORE),                  # another action's checkpoint is next
    (_Recorded([HOOK0], OWNER0), BEFORE),
    (_Recorded([PLAY0], OWNER0, full=False), BEFORE),      # a checkpoint with no state proves nothing
    (_Recorded([PLAY0], OWNER0), PENDING),                 # the engine is parked on a choice
])
def test_no_evidence_is_no_cancellation(agrees, checks, state):
    assert not eval_suite._play_cancelled_by_the_game(checks, state)
    if checks is not None:
        assert checks.done == []


# ---- only a card Rust offers in no form can have been cancelled -----------


def _hand(*cards):
    return {'player': {}, 'piles': {'hand': [
        {'id': name, 'uid': uid, 'upgrade': upgrade} for name, uid, upgrade in cards]}}


HAND = _hand(('BODYGUARD', 3, 0), ('BODYGUARD', 4, 0), ('BODYGUARD', 5, 1), ('REAVE', 6, 0))


def test_a_card_rust_offers_in_no_form_is_unplayable():
    legal = [{'kind': 'play', 'uid': 6, 'target': 0}, {'kind': 'play', 'uid': 5}, {'kind': 'end'}]
    assert eval_suite._card_is_unplayable(HAND, legal, 3)


@pytest.mark.parametrize('offered', [
    {'kind': 'play', 'uid': 3},                       # plainly
    {'kind': 'play', 'uid': 3, 'target': 1},          # with some target
    {'kind': 'play', 'uid': 3, 'selection': 6},       # with some selection
    {'kind': 'play', 'uid': 4},                       # an identical copy stands for it
])
def test_a_card_rust_offers_in_any_form_is_playable(offered):
    # The recorded play then differs from what Rust offers: a divergence,
    # never a cancellation (test_issue3756_sole_offered_selection's
    # `test_nothing_is_guessed` is the end-to-end witness).
    assert not eval_suite._card_is_unplayable(HAND, [offered, {'kind': 'end'}], 3)


def test_an_upgraded_copy_does_not_stand_for_the_card():
    assert eval_suite._card_is_unplayable(HAND, [{'kind': 'play', 'uid': 5}], 3)


def test_a_uid_not_in_hand_is_never_unplayable():
    assert not eval_suite._card_is_unplayable(HAND, [], 9)
