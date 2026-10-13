"""#2961: recorded-line replay on LYE3ZK9FYKKV.

Two replay-layer gaps left 21 of that run's 23 fights with only the raw
capture ("Capture ends - outcome not recorded"):

* a recorded physical choice in a generic frame selection (Headbutt's
  discard -> top of draw) had no mapping unless the frame was "upgrade one";
  the matcher now asks Rust to decode each candidate ordinal
  (`describe_selection`, `engine::selected_card_uids`);
* Chosen Cheese's post-combat `GainMaxHp` heals 1, which the review's
  recorded outcome did not remove from `.run` HP. That half, and the review
  that needed it, were deleted with the Python review path (2026-10-11).
"""

import pytest

import rust_replay as replay

_HEADBUTT = ['select', 'discard', 1, 1, None, ['move', 'draw', 'top']]


def _select(index):
    return {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}


class _Session:
    """`legal`, `load` and describing `apply` over a fixed decode table."""

    def __init__(self, decoded, refused=(), broken=False):
        self.decoded, self.refused, self.broken = decoded, set(refused), broken
        self.loads = 0

    def ask(self, request):
        if request == {'cmd': 'legal'}:
            return {'actions': [_select(i) for i in range(len(self.decoded))] + [{'kind': 'end'}]}
        if request['cmd'] == 'load':
            self.loads += 1
            return {'digest': 'x'}
        if request['cmd'] == 'resolve_selection':
            # Rust's in-process rule (#3125) declines here, so these tests
            # keep covering the per-answer decode loop it falls back to.
            return {'action': None}
        assert request['cmd'] == 'apply' and request['describe_selection'] is True
        index = request['action']['answer']['index']
        if self.broken:
            raise ValueError('Rust replay time budget exceeded')
        if index in self.refused:
            raise ValueError("Rust replay refused: {'refusal': 'x'}")
        return {'ok': True, 'selected_uids': self.decoded[index]}


_BEFORE = {'player': {'pending': ['frame_select', 13, _HEADBUTT]},
           'piles': {'discard': [{'uid': 4, 'id': 'BASH'}, {'uid': 9, 'id': 'STRIKE_IRONCLAD'}],
                     'play': [{'uid': 13, 'id': 'HEADBUTT'}]}}
_CHOICE = {'type': 'CombatCard', 'combat_card_indexes': [9]}


def test_generic_frame_selection_maps_through_the_rust_decode():
    session = _Session([[4], [9]])
    assert replay._selection(session, _BEFORE, _CHOICE, lambda index: index) == _select(1)
    # Every candidate is decoded from the same root, and the root is restored.
    assert session.loads == 3


def test_a_candidate_rust_refuses_is_not_the_answer():
    assert replay._selection(_Session([[4], [9]], refused={0}), _BEFORE, _CHOICE,
                             lambda index: index) == _select(1)


def test_transport_failure_while_decoding_still_raises():
    with pytest.raises(ValueError, match='time budget'):
        replay._selection(_Session([[4], [9]], broken=True), _BEFORE, _CHOICE, lambda index: index)


@pytest.mark.parametrize('decoded', [
    [[4], [4]],        # the recorded card is offered by no ordinal
    [[9], [9]],        # two ordinals claim it: no unique answer
    [[4], None],       # a nonphysical decode names nothing
])
def test_no_unique_decoded_answer_refuses(decoded):
    with pytest.raises(ValueError, match='no unique supported Rust answer'):
        replay._selection(_Session(decoded), _BEFORE, _CHOICE, lambda index: index)
