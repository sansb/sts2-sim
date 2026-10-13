"""#3756: a recorded choice of no card, and a choice the engine refuses.

Two things the census reported as `selection: recorded selection ...`:

* A `PlayerChoice` whose card list is EMPTY. The game records the player's
  whole answer, so a selection with a minimum of zero confirmed with nothing
  picked (Gambling Chip, an "up to N" pick) is `combat_card_indexes: []`
  (`rust_replay._selection` carries the IL). The resolver refused any list
  that was not exactly one card; it now asks the engine for the one offered
  answer naming exactly the recorded cards, whatever their number.
* A choice the engine itself refuses to apply. The resolver finds no answer,
  and the census now adds the engine's own refusal to the detail.

Every fixture is synthetic.
"""
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "engine" / "tools"))

import eval_suite  # noqa: E402
import rust_replay  # noqa: E402


def _select(index):
    return {"kind": "select", "answer": {"kind": "option_index", "index": index}}


class _Engine:
    """`legal` and `resolve_selection` over a fixed table of decodes."""

    def __init__(self, decoded):
        self.decoded = decoded
        self.asked = []

    def ask(self, request):
        self.asked.append(request)
        if request == {"cmd": "legal"}:
            return {"actions": [_select(i) for i in range(len(self.decoded))]}
        if request["cmd"] == "resolve_selection":
            hits = [i for i, uids in enumerate(self.decoded)
                    if uids == request["uids"]]
            return {"action": _select(hits[0]) if len(hits) == 1 else None}
        raise AssertionError(request)


CHIP = {"player": {"pending": ["gambling_chip_select", []]},
        "piles": {"hand": [{"uid": 3, "id": "A"}, {"uid": 4, "id": "B"}]}}


def _choice(indexes):
    return {"type": "CombatCard", "combat_card_indexes": indexes}


def _identity(index):
    return index


def test_a_choice_of_no_card_is_the_engines_one_empty_answer():
    engine = _Engine([[], [3], [4], [3, 4], [4, 3]])
    assert rust_replay._selection(engine, CHIP, _choice([]), _identity) \
        == _select(0)
    assert engine.asked[-1] == {"cmd": "resolve_selection", "uids": []}


def test_a_choice_of_several_cards_is_resolved_in_order():
    engine = _Engine([[], [3], [4], [3, 4], [4, 3]])
    assert rust_replay._selection(engine, CHIP, _choice([4, 3]), _identity) \
        == _select(4)
    assert rust_replay._selection(engine, CHIP, _choice([3]), _identity) \
        == _select(1)


def test_an_empty_choice_the_engine_does_not_offer_is_refused():
    """The control: an empty list is not waved through. A selection that
    requires a card offers no answer naming none, and neither does a state
    with nothing to choose."""
    for decoded in ([[3], [4]], []):
        with pytest.raises(ValueError, match="no unique supported Rust answer"):
            rust_replay._selection(_Engine(decoded), CHIP, _choice([]),
                                   _identity)
    # Nor is a multi-card choice the engine cannot name handed to the
    # single-card fallback.
    with pytest.raises(ValueError, match="no unique supported Rust answer"):
        rust_replay._selection(_Engine([[3], [4]]), CHIP, _choice([3, 4]),
                               _identity)


def test_a_malformed_card_list_is_refused_by_name():
    for indexes in (None, 3, ["3"], [3, None]):
        with pytest.raises(ValueError, match="list of physical cards"):
            rust_replay._selection(_Engine([[]]), CHIP, _choice(indexes),
                                   _identity)


class _Restoring:
    """The slice of `_RestoringSession` the refusal probe touches."""

    def __init__(self, legal, response):
        self.session = self
        self._legal, self._response = legal, response
        self.restored = 0

    def restore(self):
        self.restored += 1

    def ask(self, request):
        assert request == {"cmd": "legal"}
        return {"actions": self._legal}

    def request(self, request):
        assert request == {"cmd": "apply", "action": self._legal[0]}
        return self._response


def test_the_census_names_the_engines_refusal_of_its_own_answer():
    refusal = {"refusal": {"detail": "a continuation frame is not resumable "
                                     "in this slice",
                           "kind": "engine_not_implemented", "site": "apply"}}
    probe = _Restoring([_select(0), _select(1)], refusal)
    assert eval_suite._offered_answer_refusal(probe) == (
        "a continuation frame is not resumable in this slice")
    assert probe.restored == 1  # probed on the state the choice was made in
    # An answer the engine applies, and a state that offers none, say nothing.
    assert eval_suite._offered_answer_refusal(
        _Restoring([_select(0)], {"ok": {"digest": "d"}})) == ""
    assert eval_suite._offered_answer_refusal(
        _Restoring([{"kind": "end"}], refusal)) == ""
    # A refusal with no detail still names its kind.
    assert eval_suite._offered_answer_refusal(
        _Restoring([_select(0)], {"refusal": {"kind": "invalid_action"}})
    ) == "invalid_action"


class _ParkedSession:
    """An engine parked on a choice whose every answer it refuses."""

    state = {"player": {"turn": 1, "hp": 40, "pending": ["frame_select", 9]},
             "monsters": [], "piles": {"hand": [{"uid": 3, "id": "A"}]}}

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def request(self, request):
        if request["cmd"] == "apply":
            return {"refusal": {"detail": "malformed argument tuple at "
                                          "stratagem selection"}}
        return {"ok": self.ask(request)}

    def ask(self, request):
        if request["cmd"] == "legal":
            return {"actions": [_select(0)]}
        if request["cmd"] == "load":
            return {"digest": "root"}
        if request["cmd"] == "resolve_selection":
            return {"action": None}
        if request["cmd"] == "apply":
            raise ValueError("Rust replay refused: {'refusal': 'x'}")
        raise AssertionError(request)


def test_a_refused_choice_diverges_with_the_engines_reason(monkeypatch):
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    session = _ParkedSession()
    events = [{"event_type": "PlayerChoice", "choice_id": 0,
               "result": _choice([3])}]
    with pytest.raises(eval_suite.LineDiverged) as caught:
        eval_suite.replay_recorded_line(
            session, session.state, {"events": events, "checksums": []})
    assert caught.value.check == "selection"
    assert caught.value.detail == (
        "recorded selection has no unique supported Rust answer; Rust "
        "refuses the answers it offers: malformed argument tuple at "
        "stratagem selection")
