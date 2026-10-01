"""#3125: a recorded physical choice resolves through Rust's in-process rule.

Gambling Chip offers every ordered discard subset (326 answers for a 5-card
opening hand) and a 3-of-8 frame pick offers 336, both past the replay's
128-answer wire budget, so those fights stopped at the choice with "recorded
selection exceeds replay option budget". `rust_replay._selection` now asks
`resolve_selection` (`engine::recorded_selection_answer`) first: Rust decodes
each offered answer through the transition's own enumerator, keeps the one
naming exactly the recorded card that also applies, and refuses when two do.
"""
import pytest

import rust_replay as replay

_CHIP = {'player': {'pending': ['gambling_chip_select', [[0, ['DEFEND_IRONCLAD', 0]],
                                                         [1, ['BREAKTHROUGH', 0]]]]},
         'piles': {'hand': [{'uid': 0, 'id': 'DEFEND_IRONCLAD'}, {'uid': 1, 'id': 'BREAKTHROUGH'}]}}
_CHOICE = {'type': 'CombatCard', 'combat_card_indexes': [1]}


def _select(index):
    return {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}


class _Session:
    """326 offered selects; `resolve_selection` answers from a fixed reply."""

    def __init__(self, resolved=None, refusal=None, decoded=None):
        self.resolved, self.refusal, self.decoded = resolved, refusal, decoded
        self.requests = []

    def ask(self, request):
        self.requests.append(request)
        if request == {'cmd': 'legal'}:
            return {'actions': [_select(i) for i in range(326)] + [{'kind': 'end'}]}
        if request['cmd'] == 'resolve_selection':
            if self.refusal:
                raise ValueError(f"Rust replay refused: {{'refusal': '{self.refusal}'}}")
            return {'action': self.resolved}
        if request['cmd'] == 'load':
            return {'digest': 'x'}
        assert request['cmd'] == 'apply' and request['describe_selection'] is True
        return {'selected_uids': self.decoded[request['action']['answer']['index']]}


def test_an_ordered_surface_past_the_budget_resolves_through_rust():
    session = _Session(resolved=_select(2))
    assert replay._selection(session, _CHIP, _CHOICE, lambda index: index) == _select(2)
    # One read-only request; no per-answer load/apply round trips.
    assert session.requests == [{'cmd': 'legal'}, {'cmd': 'resolve_selection', 'uids': [1]}]


def test_a_rust_ambiguity_refusal_is_not_swallowed():
    session = _Session(refusal='recorded selection names more than one answer')
    with pytest.raises(ValueError, match='more than one answer'):
        replay._selection(session, _CHIP, _CHOICE, lambda index: index)


def test_no_rust_answer_still_meets_the_budget_by_name():
    # Null from Rust falls through to the pile-position rules, which keep
    # their per-answer budget: nothing is enumerated past it.
    session = _Session(resolved=None)
    with pytest.raises(ValueError, match='exceeds replay option budget'):
        replay._selection(session, _CHIP, _CHOICE, lambda index: index)
    assert not any(r['cmd'] in ('load', 'apply') for r in session.requests)


def test_no_rust_answer_under_the_budget_uses_the_existing_decode_loop():
    before = {'player': {'pending': ['frame_select', 13, ['select', 'discard', 1, 1, None,
                                                          ['move', 'draw', 'top']]]},
              'piles': {'discard': [{'uid': 4, 'id': 'BASH'}, {'uid': 9, 'id': 'STRIKE_IRONCLAD'}]}}

    class Small(_Session):
        def ask(self, request):
            if request == {'cmd': 'legal'}:
                return {'actions': [_select(0), _select(1), {'kind': 'end'}]}
            return super().ask(request)

    session = Small(resolved=None, decoded=[[4], [9]])
    assert replay._selection(session, before, {'type': 'CombatCard', 'combat_card_indexes': [9]},
                             lambda index: index) == _select(1)
