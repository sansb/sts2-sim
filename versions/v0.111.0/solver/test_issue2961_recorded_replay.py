"""#2961: recorded-line replay on LYE3ZK9FYKKV.

Two replay-layer gaps left 21 of that run's 23 fights with only the raw
capture ("Capture ends - outcome not recorded"):

* a recorded physical choice in a generic frame selection (Headbutt's
  discard -> top of draw) had no mapping unless the frame was "upgrade one";
  the matcher now asks Rust to decode each candidate ordinal
  (`describe_selection`, `engine::selected_card_uids`);
* Chosen Cheese's post-combat `GainMaxHp` heals 1, which `actual_outcome`
  did not remove from `.run` HP, so every fight after floor 20 missed the
  recorded result by exactly 1.
"""
import json
from pathlib import Path

import pytest

import mcr_parser
import review_summary as review
import rust_replay as replay
import rust_review

TESTDATA = Path(__file__).parent / 'testdata'
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


def _fight(hp_after, hp_healed, relics, max_hp=100, entering=80):
    return type('Fight', (), {
        'hp_entering': entering, 'max_hp_entering': max_hp,
        'hp_after': hp_after, 'hp_healed': hp_healed,
        'relics_entering': relics, 'potions_used': [], 'turns_taken': 5})()


def test_actual_outcome_removes_chosen_cheese_heal():
    actual = review.actual_outcome(_fight(61, 1, ['RELIC.CHOSEN_CHEESE']))
    assert (actual['final_hp'], actual['hp_lost']) == (60, 20)


def test_actual_outcome_composes_cheese_with_black_blood():
    # LYE3ZK9FYKKV floor 43: .run 27 = combat 14 + Black Blood 12 + Cheese 1.
    actual = review.actual_outcome(_fight(
        27, 23, ['RELIC.BLACK_BLOOD', 'RELIC.CHOSEN_CHEESE'], max_hp=154, entering=84))
    assert actual['final_hp'] == 14


def test_cheese_raised_cap_still_refuses_an_ambiguous_black_blood_heal():
    # Black Blood capped at the entering max, then Cheese lifted both by 1.
    with pytest.raises(review.ReviewRefusal) as raised:
        review.actual_outcome(_fight(101, 13, ['RELIC.BLACK_BLOOD', 'RELIC.CHOSEN_CHEESE']))
    assert raised.value.reason == 'observed_final_hp_ambiguous'


def test_a_loss_is_not_adjusted_for_cheese():
    actual = review.actual_outcome(_fight(0, 0, ['RELIC.CHOSEN_CHEESE']))
    assert (actual['won'], actual['final_hp']) == (False, 0)


# --- the real captures through the release binary --------------------------

@pytest.fixture(scope='module')
def lye3():
    return json.loads((TESTDATA / 'LYE3ZK9FYKKV_recorded_replay_2961.json').read_text())


def _binary():
    binary = rust_review.default_binary()
    if binary is None:
        pytest.skip('set STS_SIM_EXACT_SOLVER to run native replay integration')
    return binary


@pytest.mark.parametrize('floor,actions,checkpoints', [
    ('4', 14, 10),     # Headbutt only
    ('21', 14, 11),    # Headbutt + Chosen Cheese
    ('43', 23, 18),    # Headbutt + Cheese + Black Blood, Horn/Thorns (#2927)
])
def test_lye3_recorded_line_replays_completely(lye3, floor, actions, checkpoints):
    fight = lye3['fights'][floor]
    capture = mcr_parser.McrDecoder((TESTDATA / fight['mcr']).read_bytes()).combat_replay()
    witness = replay.recorded_witness(_binary(), fight['entry'], capture, fight['actual'])
    assert len(witness['actions']) == actions
    assert witness['native_checkpoints'] == checkpoints
    assert witness['won'] is True
    assert witness['combat_hp'] == fight['actual']['final_hp']
