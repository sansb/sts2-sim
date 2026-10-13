"""#3756: an input recorded while a turn-start hook's choice is open.

The game records a player action when it is enqueued. A hook that needs a
choice pauses at the front of the player's queue, so a potion enqueued behind
it runs after the hook, and a card play enqueued behind a HAND selection is
cancelled outright (`rust_replay.input_behind_open_choice` carries the IL).
The census's `eval_suite.replay_recorded_line` is held to that here, over a
scripted engine session. The Rust port's rule is tested in
`engine/src/recorded.rs`.

Every fixture is synthetic.
"""
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "engine" / "tools"))

import eval_suite  # noqa: E402
import rust_replay  # noqa: E402

SELECT = {"kind": "select", "answer": {"kind": "option_index", "index": 0}}
HOOK_CHECKPOINT = ("finished action execution GenericHookGameAction id 0 "
                   "owner 1 source  last involved ")
POTION_CHECKPOINT = ("finished action execution UsePotionAction 1  index: 0 "
                     "target: 0 (Creature) combat: True")
PLAY_CHECKPOINT = ("finished action execution PlayCardAction card: "
                   "CARD.STRIKE (1) index: 5 targetid: ")


class _HookSession:
    """Rust parked on one hook's choice, holding a potion and a Strike.

    `second` is a modal the select opens instead of clearing `pending`.
    """

    def __init__(self, hook, second=None):
        self.state = {"player": {"turn": 1, "hp": 40, "pending": [hook]},
                      "monsters": [],
                      "piles": {"hand": [{"uid": 5, "id": "STRIKE"}]}}
        self.second = second
        self.applied = []

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        command = request["cmd"]
        if command == "legal":
            if self.state["player"].get("pending"):
                return {"actions": [SELECT]}
            return {"actions": [{"kind": "potion", "slot": 0},
                                {"kind": "play", "uid": 5}, {"kind": "end"}]}
        if command == "apply":
            kind = request["action"]["kind"]
            player = dict(self.state["player"])
            if kind == "select":
                if self.second:
                    player["pending"], self.second = [self.second], None
                else:
                    player.pop("pending")
            elif player.get("pending"):
                raise ValueError(f"Rust replay refused: {kind} while pending")
            if kind == "end":
                player["over"] = True
            self.applied.append(kind)
            self.state = dict(self.state, player=player)
            return {"digest": "d", "native_checkpoints": []}
        if command == "project":
            return {"state": self.state}
        raise AssertionError(request)


def _hook():
    return {"event_type": "HookAction", "hook_id": 0,
            "game_action_type": "CombatPlayPhaseOnly"}


def _potion():
    return {"event_type": "GameAction", "action": {
        "type": "NetUsePotionAction", "potion_index": 0}}


def _play():
    return {"event_type": "GameAction", "action": {
        "type": "NetPlayCardAction", "combat_card_index": 5,
        "card_id": "CARD.STRIKE"}}


def _choice():
    return {"event_type": "PlayerChoice", "choice_id": 0, "result": {
        "type": "CombatCard", "combat_card_indexes": [5]}}


def _resume():
    return {"event_type": "ResumeAction", "action_id": 0}


def _end():
    return {"event_type": "GameAction", "action": {
        "type": "NetEndPlayerTurnAction", "turn_number": 1}}


def _checksums(*contexts):
    return [{"checksum": {"id": i, "value": 0}, "context": context,
             "full_state": {"creatures": []}}
            for i, context in enumerate(contexts)]


@pytest.fixture
def census(monkeypatch):
    """`replay_recorded_line` over a `_HookSession`, checkpoints agreeing on
    state so that only their ORDER and OWNERS are under test."""
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: None)
    monkeypatch.setattr(eval_suite, "power_mismatch_rows",
                        lambda state, native: [])
    monkeypatch.setattr(rust_replay, "_selection", lambda *a: dict(SELECT))

    def run(hook, events, contexts, second=None):
        session = _HookSession(hook, second)
        line = eval_suite.replay_recorded_line(
            session, session.state,
            {"events": events, "checksums": _checksums(*contexts)})
        return line, session
    return run


HAND_HOOKS = ("gambling_chip_select", "toasty_mittens_select",
              "turn_start_hand_choice_select")


def test_the_rule_names_each_hook_choice_and_nothing_else():
    def behind(pending, kind):
        return rust_replay.input_behind_open_choice(
            {"player": {"pending": pending}}, kind)
    for hook in HAND_HOOKS:
        assert behind([hook], "NetPlayCardAction") == "cancelled"
        assert behind([hook], "NetUsePotionAction") == "queued"
    assert behind(["stratagem_select"], "NetPlayCardAction") == "queued"
    assert behind(["stratagem_select"], "NetUsePotionAction") == "queued"
    # No open choice, a choice no hook opened, and an input that is neither.
    for pending in ([], None, ["frame_select", 3], ["potion_select", "X"],
                    ["foregone_select"], ["discovery_select"]):
        for kind in ("NetPlayCardAction", "NetUsePotionAction"):
            assert behind(pending, kind) is None
    for kind in ("NetEndPlayerTurnAction", "NetReadyToBeginEnemyTurnAction",
                 None):
        assert behind(["gambling_chip_select"], kind) is None


@pytest.mark.parametrize("hook", HAND_HOOKS + ("stratagem_select",))
def test_a_potion_enqueued_behind_a_hook_choice_runs_after_it(
        census, hook):
    """The Gambling Chip and Stratagem shape: hook, potion, the hook's
    choice. The game answers the choice, finishes the hook, then drinks."""
    events = [_hook(), _potion(), _choice(), _resume(), _end()]
    contexts = [HOOK_CHECKPOINT, POTION_CHECKPOINT]
    line, session = census(hook, events, contexts)
    assert session.applied == ["select", "potion", "end"]
    assert [a["kind"] for a in line["actions"]] == ["select", "potion", "end"]
    assert line["checkpoint_mismatch"] is None
    assert line["native_checkpoints"] == 2
    assert line["cancelled_inputs"] == 0


@pytest.mark.parametrize("hook", HAND_HOOKS)
def test_a_card_play_enqueued_behind_a_hand_choice_never_runs(
        census, hook):
    """The Tyranny shape: the play is in the event list and owns no
    checkpoint, because the open hand selection cancelled it."""
    events = [_hook(), _play(), _choice(), _resume(), _end()]
    line, session = census(hook, events, [HOOK_CHECKPOINT])
    assert session.applied == ["select", "end"]
    assert line["checkpoint_mismatch"] is None
    assert (line["native_checkpoints"], line["cancelled_inputs"]) == (1, 1)


@pytest.mark.parametrize("hook", HAND_HOOKS)
def test_a_play_the_game_did_run_is_not_passed_off_as_cancelled(
        census, hook):
    """The control: the capture holds the play's own completed checkpoint,
    so the game ran it. Dropping it leaves that checkpoint unconsumed, and
    the resolver says so."""
    events = [_hook(), _play(), _choice(), _resume(), _end()]
    contexts = [HOOK_CHECKPOINT, PLAY_CHECKPOINT]
    line, _session = census(hook, events, contexts)
    assert "unconsumed" in line["checkpoint_mismatch"]["detail"]


def test_a_card_play_enqueued_behind_a_pile_choice_runs_after_it(
        census):
    """Stratagem selects from the draw pile, which cancels nothing: the play
    waits in the queue, behind an earlier potion."""
    events = [_hook(), _potion(), _play(), _choice(), _resume(), _end()]
    contexts = [HOOK_CHECKPOINT, POTION_CHECKPOINT, PLAY_CHECKPOINT]
    line, session = census("stratagem_select", events, contexts)
    assert session.applied == ["select", "potion", "play", "end"]
    assert line["checkpoint_mismatch"] is None
    assert (line["native_checkpoints"], line["cancelled_inputs"]) == (3, 0)


def test_a_queued_play_the_game_never_ran_is_a_mismatch(census):
    """The control for the queued branch: no checkpoint for the play."""
    events = [_hook(), _play(), _choice(), _resume(), _end()]
    line, _session = census("stratagem_select", events, [HOOK_CHECKPOINT])
    assert "checkpoint missing" in line["checkpoint_mismatch"]["detail"]


def test_an_input_behind_any_other_choice_is_still_not_legal(census):
    """A choice opened by a card or a potion is not read: as before."""
    events = [_hook(), _potion(), _choice(), _resume(), _end()]
    with pytest.raises(eval_suite.LineDiverged) as caught:
        census("frame_select", events, [HOOK_CHECKPOINT])
    assert (caught.value.check, caught.value.step) == ("not_legal", 0)


def test_a_waiting_input_is_never_applied_out_of_order(census):
    """Two refusals, by name. The select opens a second choice (natively the
    potion may run before that hook is even enqueued), or the capture never
    answers the first."""
    events = [_hook(), _potion(), _choice(), _resume(), _end()]
    with pytest.raises(eval_suite.LineDiverged) as caught:
        census("gambling_chip_select", events, [HOOK_CHECKPOINT],
               second="turn_start_hand_choice_select")
    assert caught.value.check == "not_legal"
    assert "input 1 waits behind a second open choice" in caught.value.detail

    unanswered = [_hook(), _potion()]
    with pytest.raises(eval_suite.LineDiverged) as caught:
        census("gambling_chip_select", unanswered, [])
    assert caught.value.check == "not_legal"
    assert "never answers" in caught.value.detail
