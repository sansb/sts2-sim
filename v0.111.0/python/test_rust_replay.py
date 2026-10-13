"""Recorded-input hydration, including physical choices and safe fallback."""
import copy
import json
from pathlib import Path

import pytest
import rust_replay as replay
from rust_exact_solve import default_binary


@pytest.fixture
def queen():
    return json.loads((Path(__file__).parent / 'testdata/rust_review_queen.json').read_text())


def test_mittens_choice_names_the_physical_card_and_reason(queen):
    details = replay.selection_details(queen['entry'], queen['expected'][0]['action'])
    assert details == {'uids': [6], 'cards': ['HOWL_FROM_BEYOND+'],
                       'selection_operation': 'exhaust', 'selection_source': 'TOASTY_MITTENS'}


def test_generation_choice_uses_offered_card_name():
    before = {'player': {'pending': ['generation_potion_select', 'ATTACK_POTION', [['THRASH', 0], ['SPITE', 1]]]}}
    details = replay.selection_details(before, {'answer': {'index': 1}})
    assert details['cards'] == ['SPITE+']
    assert details['selection_source'] == 'ATTACK_POTION'
    assert replay.selection_details(before, {'answer': {'index': -1}}) is None


def test_foregone_choice_labels_selected_cards_and_return_to_hand():
    before = {
        'player': {'pending': ['foregone_select', ['select', 'draw', 2, 2, None,
                                                    ['move', 'hand', 'bottom']], []]},
        'piles': {'draw': [{'uid': 3, 'id': 'STRIKE_IRONCLAD'},
                           {'uid': 9, 'id': 'DEFEND_IRONCLAD'}]},
    }
    details = replay.selection_details(before, {'answer': {'index': 0}}, selected_uids=[9, 3])
    assert details == {'uids': [9, 3], 'cards': ['DEFEND_IRONCLAD', 'STRIKE_IRONCLAD'],
                       'selection_source': 'FOREGONE_CONCLUSION',
                       'selection_operation': 'return_to_hand'}


def test_upgrade_choice_tracks_uid_instead_of_hand_position():
    before = {'player': {'pending': ['frame_select', 8, ['select', 'hand', 1, 1, 'upgradable', 'upgrade']]},
              'piles': {'hand': [{'uid': 6, 'id': 'STRIKE_IRONCLAD'}, {'uid': 9, 'id': 'STRIKE_IRONCLAD'}],
                        'play': [{'uid': 8, 'id': 'ARMAMENTS'}]}}
    after = copy.deepcopy(before)
    after['piles']['hand'][1]['upgrade'] = 1
    assert replay.selection_details(before, {'answer': {'index': 0}}, after)['uids'] == [9]
    after['piles']['hand'][0]['upgrade'] = 1
    assert replay.selection_details(before, {'answer': {'index': 0}}, after) is None


def test_checkpoint_cannot_slide_to_a_different_recorded_action(queen):
    check = replay.NativeChecks(queen['replay'])
    check.start({'action': {'type': 'NetPlayCardAction', 'combat_card_index': 41}})
    with pytest.raises(ValueError, match='action identity'):
        check.completed({'player': {}})
    with pytest.raises(ValueError, match='unconsumed'):
        check.finish()


def test_native_state_mismatch_is_rejected(queen):
    native = queen['replay']['checksums'][0]['full_state']
    # The opening is before Mittens, not the completed hook checkpoint.
    with pytest.raises(ValueError, match='native'):
        replay.check_native_snapshot(queen['entry'], native)


@pytest.mark.parametrize('player', [
    {'hp': 100, 'max_hp': 100, 'energy': 20, 'player_phase': 3,
     'next_card_uid': 2, 'lightning_rod': 2, 'orb_slots': 3},
    {'hp': 100, 'max_hp': 100, 'energy': 20, 'player_phase': 3,
     'genesis': 2, 'star_next_turn': 3,
     'star_energy_reset_order': ['star_next_turn', 'genesis']},
], ids=['legacy-lightning-rod-singleton', 'legacy-star-subgroup'])
def test_rust_replay_authenticates_legacy_inferred_reset_order_roots(player):
    """Hydrating a unique legacy order may not add a wire field on load."""
    binary = default_binary()
    if binary is None:
        pytest.skip('set STS_SIM_EXACT_SOLVER to run replay integration')
    entry = json.loads((Path(__file__).parent / 'testdata'
                        / 'ea6_reset_order_replay_base.json').read_text())
    entry['player'] = player
    entry['piles'] = {'hand': [], 'draw': [], 'discard': []}
    # RustReplay.__enter__ compares the raw entry digest immediately after
    # diff-serve load. This exercises the real public replay contract rather
    # than merely asserting a boundary projection in process.
    with replay.RustReplay(binary, entry, timeout=30):
        pass


@pytest.mark.parametrize('operation,label', [
    ('exhaust', 'exhaust'), ('discard', 'discard'), ('upgrade', 'upgrade'),
    ('apply_permanent_retain', 'retain'), ('free_this_combat', 'make_free'),
    (['move', 'draw', 'top'], 'put_on_deck'),
    (['move', 'hand', 'bottom'], 'return_to_hand'),
    (['move_free_this_turn', 'hand', 'bottom'], 'return_free'),
    (['clone_generated', 1], 'copy'), (['exhaust_draw', 2], 'exhaust'),
])
def test_physical_selection_labels_use_rust_uids_in_pick_order(operation, label):
    before = {'player': {'pending': ['frame_select', 8, ['select', 'hand', 0, 2, None, operation]]},
              'piles': {'hand': [{'uid': 3, 'id': 'STRIKE_IRONCLAD'},
                                  {'uid': 9, 'id': 'STRIKE_IRONCLAD', 'upgrade': 1}],
                        'play': [{'uid': 8, 'id': 'BRAND'}]}}
    # The ordinal and pile position intentionally disagree. A suffix could
    # move either card: only the consumer's physical answer is authoritative.
    details = replay.selection_details(before, {'answer': {'index': 0}}, selected_uids=[9, 3])
    assert details['cards'] == ['STRIKE_IRONCLAD+', 'STRIKE_IRONCLAD']
    assert details['selection_source'] == 'BRAND'
    assert details['selection_operation'] == label
    assert replay.selection_details(before, {'answer': {'index': 0}}, selected_uids=[])['cards'] == []
    with pytest.raises(ValueError, match='missing'):
        replay.selection_details(before, {'answer': {'index': 0}}, selected_uids=[99])
