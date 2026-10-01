"""#3156: a recorded Knowledge Demon curse choice resolves to Rust's ordinal.

CURSE_OF_KNOWLEDGE parks the enemy turn on `CardSelectCmd::
FromChooseACardScreen` over `_curseOfKnowledgeSets[counter]` with
canSkip=false (`<ChooseCurse>d__39::MoveNext` RVA 0x3605e0, IL_0069-IL_00a2),
so the `.mcr` records an `Index` into that set. Rust projects the same set, in
the same order, as the player's `enemy_pending` and offers `option_index` k
for option k. Six census lines (all KNOWLEDGE_DEMON_BOSS, every one picking
index 0) stopped there with "no unique supported Rust answer".
"""
import pytest

import rust_replay as replay


def _select(index):
    return {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}


def _before(counter_second='MIND_ROT'):
    return {'player': {'pending': None,
                       'enemy_pending': ['KD_CURSE', ['DISINTEGRATION', counter_second], 0, []]},
            'piles': {'hand': []}}


class _Session:
    def __init__(self, offered=(0, 1)):
        self.offered = offered
        self.requests = []

    def ask(self, request):
        self.requests.append(request)
        assert request == {'cmd': 'legal'}, request
        return {'actions': [_select(i) for i in self.offered]}


def _index(*indexes):
    return {'type': 'Index', 'indexes': list(indexes)}


@pytest.mark.parametrize('index', [0, 1])
def test_each_offered_curse_resolves_to_its_own_ordinal(index):
    session = _Session()
    assert replay._selection(session, _before(), _index(index), lambda i: i) == _select(index)
    # One read-only request: nothing is trial-applied.
    assert session.requests == [{'cmd': 'legal'}]


@pytest.mark.parametrize('index', [-1, 2])
def test_a_skip_or_an_index_past_the_set_refuses_by_name(index):
    # canSkip=false (IL_00a1 ldc.i4.0): -1 cannot be a native answer.
    with pytest.raises(ValueError, match='curse index outside the offered set'):
        replay._selection(_Session(), _before(), _index(index), lambda i: i)


def test_an_answer_rust_does_not_offer_refuses_by_name():
    with pytest.raises(ValueError, match='not offered by Rust'):
        replay._selection(_Session(offered=(0,)), _before(), _index(1), lambda i: i)


def test_a_non_index_record_at_the_curse_choice_refuses():
    with pytest.raises(ValueError, match='is not an Index'):
        replay._selection(_Session(), _before(),
                          {'type': 'CombatCard', 'combat_card_indexes': [0]}, lambda i: i)


def test_a_malformed_index_list_refuses():
    with pytest.raises(ValueError, match='one display index'):
        replay._selection(_Session(), _before(), _index(0, 1), lambda i: i)


def test_a_curse_choice_without_an_option_list_refuses():
    before = {'player': {'pending': None, 'enemy_pending': ['KD_CURSE']}}
    with pytest.raises(ValueError, match='no option list'):
        replay._selection(_Session(), before, _index(0), lambda i: i)


def test_a_player_modal_takes_precedence_over_the_enemy_projection():
    # A card's own pending modal still owns the choice; the KD rule reads the
    # enemy projection only when the player has none.
    before = _before()
    before['player']['pending'] = ['generation_relic_select', 'X', [['A', 0]]]
    session = _Session()
    assert replay._selection(session, before, _index(0), lambda i: i) == _select(0)


@pytest.mark.parametrize('index,card', [(0, 'DISINTEGRATION'), (1, 'SLOTH')])
def test_review_labels_the_chosen_curse(index, card):
    assert replay.selection_details(_before('SLOTH'), _select(index)) == {
        'cards': [card], 'selection_operation': 'choose', 'selection_source': 'KNOWLEDGE_DEMON'}


def test_review_label_is_absent_for_an_unoffered_ordinal():
    assert replay.selection_details(_before(), _select(2)) is None
