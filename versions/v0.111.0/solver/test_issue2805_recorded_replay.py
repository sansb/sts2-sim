"""#2805: the Rust replay of the recorded line on YLVVPKPH1MTW.

Four refusal classes were replay-layer gaps and are fixed here:

* a recorded play of a payload-identical Hand copy that `legal` collapsed
  onto another representative (floors 5 and 33);
* Discovery's choose-a-card `Index` result (floors 9, 15, 19 and 31);
* Thieving Hopper roots, whose deck uids are master-deck rows (floor 20);
* Decimillipede's three native segment classes behind one Rust kind (floor 25).

Two engine divergences were found here and are fixed in Rust. Demon Tongue
healed enemy-side damage (#2808): floors 31 and 33 now replay completely.
Monster Plating timing (#2809) made floor 17's root and first checkpoints
match native; with Lagavulin Matriarch's sleeping side (AsleepPower's
SLEEP_MOVE, VeryEarly Plating removal and countdown wake) ported to Rust
(#2814), floor 17 now replays completely too.
"""
import copy
import json
from pathlib import Path

import pytest

import mcr_native
import mcr_parser
import rust_replay as replay
import rust_review as review

TESTDATA = Path(__file__).parent / 'testdata'


@pytest.fixture(scope='module')
def ylvv():
    return json.loads((TESTDATA / 'YLVVPKPH1MTW_recorded_replay_2805.json').read_text())


def _capture(fight):
    return mcr_parser.McrDecoder((TESTDATA / fight['mcr']).read_bytes()).combat_replay()


class _Session:
    def __init__(self, legal):
        self.legal = legal

    def ask(self, request):
        assert request == {'cmd': 'legal'}
        return {'actions': self.legal}


def _select(index):
    return {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}


# --- recorded play of a collapsed duplicate --------------------------------

_HAND = {'piles': {'hand': [
    {'uid': 5, 'id': 'DEFEND_IRONCLAD'}, {'uid': 6, 'id': 'GREED'},
    {'uid': 7, 'id': 'STRIKE_IRONCLAD'}, {'uid': 8, 'id': 'DEFEND_IRONCLAD'},
    {'uid': 9, 'id': 'STRIKE_IRONCLAD', 'upgrade': 1}]}}
_LEGAL = [{'kind': 'play', 'target': 0, 'uid': 7}, {'kind': 'play', 'uid': 5},
          {'kind': 'play', 'target': 0, 'uid': 9}, {'kind': 'end'}]


def test_collapsed_duplicate_play_is_offered_by_its_representative():
    # Floor 5 input 5: legal offers Defend uid 5; the player clicked uid 8.
    assert replay._play_is_offered(_HAND, _LEGAL, {'kind': 'play', 'uid': 8})
    assert replay._play_is_offered(_HAND, _LEGAL, {'kind': 'play', 'uid': 5})


@pytest.mark.parametrize('wire', [
    {'kind': 'play', 'uid': 8, 'target': 0},        # target the offer lacks
    {'kind': 'play', 'uid': 6},                      # unplayable curse
    {'kind': 'play', 'uid': 12},                     # not in hand
    {'kind': 'play', 'uid': 8, 'selection': 5},      # extra selection field
])
def test_play_outside_every_offered_group_is_refused(wire):
    assert not replay._play_is_offered(_HAND, _LEGAL, wire)


def test_upgraded_copy_is_not_interchangeable_with_the_base_card():
    legal = [{'kind': 'play', 'target': 0, 'uid': 7}, {'kind': 'end'}]
    assert not replay._play_is_offered(_HAND, legal, {'kind': 'play', 'uid': 9, 'target': 0})


def test_combined_selection_matches_modulo_duplicates():
    hand = {'piles': {'hand': [{'uid': 1, 'id': 'BURNING_PACT'}, {'uid': 2, 'id': 'WOUND'},
                               {'uid': 3, 'id': 'WOUND'}]}}
    legal = [{'kind': 'play', 'uid': 1, 'selection': 2}, {'kind': 'end'}]
    assert replay._play_is_offered(hand, legal, {'kind': 'play', 'uid': 1, 'selection': 3})
    assert not replay._play_is_offered(hand, legal, {'kind': 'play', 'uid': 1, 'selection': 1})


# --- choose-a-card Index results -------------------------------------------

_DISCOVERY = {'player': {'pending': ['discovery_select', 9, 0,
                                     [['CONFLAGRATION', 0], ['PRIMAL_FORCE', 0], ['ANGER', 0]]]}}
_DISCOVERY_LEGAL = [_select(i) for i in range(4)]


@pytest.mark.parametrize('recorded,answer', [([0], 0), ([2], 2), ([-1], 3)])
def test_discovery_index_maps_to_the_offered_option_or_skip(recorded, answer):
    result = {'type': 'Index', 'indexes': recorded}
    wire = replay._selection(_Session(_DISCOVERY_LEGAL), _DISCOVERY, result, None)
    assert wire == _select(answer)


@pytest.mark.parametrize('recorded', [[3], [-2], [], [0, 1], ['0']])
def test_discovery_index_outside_the_screen_is_refused(recorded):
    with pytest.raises(ValueError):
        replay._selection(_Session(_DISCOVERY_LEGAL), _DISCOVERY,
                          {'type': 'Index', 'indexes': recorded}, None)


def test_abundance_has_no_skip_answer():
    before = {'player': {'pending': ['abundance_select', 4, 0,
                                     [['DEMON_FORM', 1], ['INFLAME', 1], ['BARRICADE', 1]]]}}
    legal = [_select(i) for i in range(3)]
    assert replay._selection(_Session(legal), before, {'type': 'Index', 'indexes': [1]}, None) == _select(1)
    with pytest.raises(ValueError, match='outside the offered cards'):
        replay._selection(_Session(legal), before, {'type': 'Index', 'indexes': [-1]}, None)


def test_generation_potion_skip_is_the_recorded_minus_one():
    before = {'player': {'pending': ['generation_potion_select', 'ATTACK_POTION',
                                     [['THRASH', 0], ['SPITE', 1], ['ANGER', 0]]]}}
    legal = [_select(i) for i in range(4)]
    assert replay._selection(_Session(legal), before, {'type': 'Index', 'indexes': [-1]}, None) == _select(3)


# --- Thieving Hopper master-deck uids --------------------------------------

def test_hopper_rows_are_the_same_shuffle_as_the_instance_ids(ylvv):
    captured = _capture(ylvv['fights']['20'])
    rows = mcr_native.first_cycle_rows_from_replay(captured)
    deck = captured['run']['players'][0]['deck']
    assert sorted(rows) == list(range(len(deck)))
    assert rows != list(range(len(deck)))
    instances = mcr_native.first_cycle_from_replay(captured)
    assert [tuple(i) for i in instances] == [
        (deck[r]['id'].removeprefix('CARD.'),
         deck[r].get('upgrade_level') or deck[r].get('current_upgrade_level') or 0) for r in rows]
    entry = ylvv['fights']['20']['entry']
    cards = {c['uid']: c for pile in entry['piles'].values() for c in pile}
    assert all((cards[r]['id'], cards[r].get('upgrade', 0)) == tuple(instances[k][:2])
               for k, r in enumerate(rows))


def test_hopper_root_with_foreign_rows_is_refused_before_any_process(ylvv, monkeypatch):
    fight = ylvv['fights']['20']
    entry = copy.deepcopy(fight['entry'])
    entry['player']['next_card_uid'] += 1
    monkeypatch.setattr(replay.subprocess, 'Popen', lambda *a, **k: pytest.fail('must not launch'))
    with pytest.raises(ValueError, match='master-deck rows'):
        replay.recorded_witness('unused', entry, _capture(fight), fight['actual'])


def test_non_hopper_root_keeps_instance_index_uids(ylvv, monkeypatch):
    # Hopper rows applied to the floor 20 root without its marker fail the
    # identity check: the numbering is chosen by the projection, never guessed.
    fight = ylvv['fights']['20']
    entry = copy.deepcopy(fight['entry'])
    del entry['player']['hopper_master_deck']
    monkeypatch.setattr(replay.subprocess, 'Popen', lambda *a, **k: pytest.fail('must not launch'))
    with pytest.raises(ValueError, match='physical card identities'):
        replay.recorded_witness('unused', entry, _capture(fight), fight['actual'])


# --- Decimillipede segment classes -----------------------------------------

def _decimillipede_checkpoint(ylvv):
    captured = _capture(ylvv['fights']['25'])
    return next(c['full_state'] for c in captured['checksums']
                if c.get('full_state') and c['context'].startswith('finished action execution'))


def test_native_segment_classes_match_the_aggregate_rust_kind(ylvv):
    native = _decimillipede_checkpoint(ylvv)
    ids = [c['monster_id'] for c in native['creatures'] if c.get('monster_id')]
    assert ids == ['DECIMILLIPEDE_SEGMENT_FRONT', 'DECIMILLIPEDE_SEGMENT_MIDDLE',
                   'DECIMILLIPEDE_SEGMENT_BACK']
    rust = [{'kind': 'DECIMILLIPEDE_SEGMENT', 'hp': m['current_hp'], 'max_hp': m['max_hp'],
             'block': m['block']} for m in native['creatures'] if m.get('monster_id')]
    # The alias renames the class only: HP stays positional, and a Rust kind
    # spelled as one native class is not silently widened to the others.
    renamed = [dict(m, kind='DECIMILLIPEDE_SEGMENT_FRONT') for m in rust]
    swapped = copy.deepcopy(rust)
    swapped[0]['hp'], swapped[1]['hp'] = swapped[1]['hp'], swapped[0]['hp']
    player = next(c for c in native['creatures'] if c.get('player_id') is not None)
    p = native['players'][0]
    state = {'player': {'hp': player['current_hp'], 'max_hp': player['max_hp'], 'block': player['block'],
                        'energy': p['energy'], 'turn': p['turn_number']}, 'piles': {}, 'rng': {}}
    reached = {}
    for name, monsters in (('aggregate', rust), ('renamed', renamed), ('swapped', swapped)):
        with pytest.raises(ValueError) as caught:
            replay.check_native_snapshot(dict(state, monsters=monsters), native)
        reached[name] = str(caught.value)
    # The empty piles are the next check after monsters: reaching it proves
    # the monster rows matched.
    assert reached == {'aggregate': 'recorded replay differs from native card piles',
                       'renamed': 'recorded replay differs from native monsters',
                       'swapped': 'recorded replay differs from native monsters'}


# --- the real captures through the release binary --------------------------

def _binary():
    binary = review.default_binary()
    if binary is None:
        pytest.skip('set STS_SIM_EXACT_SOLVER to run native replay integration')
    return binary


@pytest.mark.parametrize('floor,actions,checkpoints', [
    ('5', 10, 8),      # duplicate Defend copy at input 5
    ('9', 12, 8),      # Discovery Index
    ('17', 34, 25),    # Plating timing (#2809) + Lagavulin's sleeping side (#2814)
    ('20', 21, 15),    # Thieving Hopper master-deck uids
    ('25', 33, 25),    # Decimillipede segment classes
    ('31', 16, 12),    # Demon Tongue owner-side gate (#2808)
    ('33', 36, 26),    # Demon Tongue owner-side gate (#2808); a recorded loss
])
def test_ylvv_recorded_line_replays_completely(ylvv, floor, actions, checkpoints):
    fight = ylvv['fights'][floor]
    witness = replay.recorded_witness(_binary(), fight['entry'], _capture(fight), fight['actual'])
    assert len(witness['actions']) == actions
    assert witness['native_checkpoints'] == checkpoints
    assert witness['won'] == fight['actual']['won']
    assert witness['combat_hp'] == fight['actual']['final_hp']


# --- monster Plating in the Rust-bound opening (#2809) ---------------------

def test_ylvv_lagavulin_root_carries_the_native_opening_block(ylvv):
    fight = ylvv['fights']['17']
    first = _capture(fight)['checksums'][0]
    assert first['context'] == 'After player turn start'
    native = [c for c in first['full_state']['creatures'] if c.get('monster_id')]
    assert [(m['current_hp'], m['block']) for m in native] == [(233, 12)]
    assert [(m['hp'], m.get('block', 0), m['mplating']) for m in fight['entry']['monsters']] == [(233, 12, 12)]
