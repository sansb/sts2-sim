"""#3242: a native checkpoint INSIDE one Rust wire action is compared at its
own boundary (`diff-serve` apply `native_checkpoints`), never skipped.

Void Form's OnPlay only marks the player ready to end the turn
(`PlayerCmd::EndTurn` RVA 0x13362f IL_0029), so its PlayCardAction checkpoint
precedes the EndTurn Rust runs in the same apply. The AfterAutoPostPlayPhase
dispatch (Stampede's AutoPlay, `StampedePower/<AfterAutoPostPlayPhaseEntered>
d__4` RVA 0x345e94 IL_00bf) runs as one GenericHookGameAction inside the `end`
wire. The end-to-end witnesses are the census rows 70022CD6G3J0 n16 and
KD13JGCDPB3U n39; these pin the production `NativeChecks` rules.
"""
import pytest
import rust_replay as replay

PLAY0 = 'finished action execution PlayCardAction card: X index: 0 targetid: '
HOOK0 = 'finished action execution GenericHookGameAction id 0 owner 1 source  last involved '
PLAY = {'action': {'type': 'NetPlayCardAction', 'combat_card_index': 0}}
HOOK = {'event_type': 'HookAction', 'hook_id': 0}
READY = {'event_type': 'GameAction', 'action': {'type': 'NetReadyToBeginEnemyTurnAction'}}
STAGED = {'player': {'turn': 1, 'marker': 'staged'}}
AFTER = {'player': {'turn': 2, 'marker': 'after'}}


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


def test_void_form_play_is_compared_before_its_requested_end_turn(compared):
    checks = _checks(PLAY0)
    checks.start(PLAY)
    played = checks.stage(_applied({'kind': replay.VOID_FORM_END_TURN_REQUEST, 'state': STAGED}))
    assert played is STAGED
    checks.completed(played if played is not None else AFTER)
    checks.finish()
    assert compared == [STAGED] and checks.validated == 1


def test_auto_post_hook_checkpoint_is_claimed_by_the_following_hook_action(compared):
    checks = _checks(HOOK0)
    assert checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED})) is None
    checks.start(HOOK)
    checks.no_decision(HOOK)
    checks.no_decision(READY)
    checks.finish()
    assert compared == [STAGED] and checks.owner is None


def test_hook_action_without_a_staged_state_keeps_its_owner_for_the_choice(compared):
    checks = _checks(HOOK0)
    checks.stage(_applied())
    checks.start(HOOK)
    checks.no_decision(HOOK)
    assert compared == [] and checks.owner is not None
    checks.completed(AFTER)  # the PlayerChoice's wire consumes it, as before
    assert compared == [AFTER]


def test_ready_marker_and_next_apply_drop_an_unclaimed_auto_post_state(compared):
    checks = _checks(HOOK0)
    checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED}))
    checks.no_decision(READY)
    checks.start(HOOK)
    checks.no_decision(HOOK)
    assert compared == []
    checks.stage(_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': STAGED}))
    checks.stage(_applied())
    checks.no_decision(HOOK)
    assert compared == []


@pytest.mark.parametrize('applied, words', [
    ({'digest': 'd'}, 'did not report'),
    (_applied({'kind': 'mystery', 'state': STAGED}), 'unknown native checkpoint kind'),
    (_applied({'kind': replay.VOID_FORM_END_TURN_REQUEST, 'state': STAGED},
              {'kind': replay.VOID_FORM_END_TURN_REQUEST, 'state': STAGED}), 'recorded twice'),
    (_applied({'kind': replay.AUTO_POST_HOOK_FINISHED, 'state': None, 'refusal': 'no'}),
     'does not project: no'),
])
def test_malformed_checkpoint_reports_refuse_by_name(applied, words):
    with pytest.raises(ValueError, match=words):
        _checks().stage(applied)
