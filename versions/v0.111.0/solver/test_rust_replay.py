"""Recorded-input hydration, including physical choices and safe fallback."""
import copy
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
import rust_replay as replay
import rust_review as review


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


def test_recorded_root_identity_mismatch_precedes_any_process(queen, monkeypatch):
    queen['entry']['piles']['hand'][0]['id'] = 'WRONG_CARD'
    monkeypatch.setattr(replay.subprocess, 'Popen', lambda *a, **k: pytest.fail('must not launch'))
    with pytest.raises(ValueError, match='physical card identities'):
        replay.recorded_witness('unused', queen['entry'], queen['replay'], {'won': False, 'final_hp': 0})


def test_only_a_whispering_earring_root_may_lack_a_dealt_card(queen, monkeypatch):
    """#3414: the Earring's turn-one loop can play a Power out of every pile
    before the root, so only there may a deck uid be absent; every present
    card is still matched."""
    class Launched(Exception):
        pass

    def launch(*a, **k):
        raise Launched
    monkeypatch.setattr(replay.subprocess, 'Popen', launch)
    witness = lambda: replay.recorded_witness(  # noqa: E731
        'unused', queen['entry'], queen['replay'], {'won': False, 'final_hp': 0})
    queen['entry']['piles']['hand'].pop(0)
    with pytest.raises(ValueError, match='physical card identities'):
        witness()
    queen['entry']['player']['relics_entering'] = list(
        queen['entry']['player'].get('relics_entering') or ()) + ['RELIC.WHISPERING_EARRING']
    with pytest.raises(Launched):
        witness()
    queen['entry']['piles']['hand'][0]['id'] = 'WRONG_CARD'
    with pytest.raises(ValueError, match='physical card identities'):
        witness()


@pytest.mark.parametrize('fails', [False, True])
def test_review_attaches_hydrated_line_or_keeps_solver_results(tmp_path, monkeypatch, fails):
    from review_summary import ReviewConfig
    entry = {'player': {'hp': 65}}
    fight = SimpleNamespace(node_index=47, encounter_id='QUEEN', node_type='boss')
    run = SimpleNamespace(build_id='v0.111.0', seed='SEED')
    monkeypatch.setattr(review, 'build_root', lambda *a: (run, fight, entry))
    best = {'won': True, 'combat_hp': 23, 'turn': 5, 'actions': [], 'final_digest': 'winner'}
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **k: SimpleNamespace(returncode=0, stdout=json.dumps({
        'entry_digest': review.canonical_document.differential_digest(entry), 'best': best})))
    monkeypatch.setattr(review.review, 'actual_outcome', lambda *a: {'entry_hp': 65, 'final_hp': 0, 'won': False})
    monkeypatch.setattr(review.review, 'simulator_document', lambda *a: {})
    monkeypatch.setattr(review, 'replay_line', lambda binary, root, witness: [{'turn': 1, 'actions': [{'digest_after': witness['final_digest']}]}])
    def hydrate(*args):
        if fails:
            raise ValueError('missing recorded action')
        return dict(best, final_digest='human', native_checkpoints=28)
    monkeypatch.setattr(review, 'recorded_witness', hydrate)
    document = review.generate(tmp_path/'run', 19, [], ReviewConfig(recorded_replay={'events': []}, rust_exact_solver_binary=tmp_path/'sim'))
    assert document['status'] == 'ok'
    if fails:
        assert 'recorded_line' not in document
        assert document['metadata']['recorded_replay']['status'] == 'unavailable'
    else:
        assert document['recorded_line'][0]['actions'][0]['digest_after'] == 'human'
        assert document['metadata']['recorded_replay']['native_checkpoints'] == 28


QUEEN_RECORDED_LINE = (Path(__file__).parents[1] / 'eval' / 'search'
                       / 'queen-capture-v1' / 'recorded-line.json')


def test_queen_pin_tracks_the_rust_lane_recorded_line(queen):
    """Binary-free drift detector for the pin below (#3263).

    The integration test runs only where a release `sts-sim` exists, and no
    CI lane builds one for the solver suite, so its digest pin drifted for a
    week unseen: #3088 refreshed `recorded-line.json` for the same capture
    (steps 7 and 27, and step 34's content) without touching this copy. That
    file IS gated — `rust port` replays it in tests/queen_capture.rs, and the
    eval tree is in the fast gate's TRIGGER_SET — so tying the two here makes
    the next refresh of one fail CI until the other follows.

    The last step is the one deliberate difference: queen_capture.rs strips
    the `player_hooks_deactivated` carrier #2638 added at the capture's actual
    player death before comparing the historical digest, while this pin
    records what `replay_line` emits, carrier included.
    """
    recorded = json.loads(QUEEN_RECORDED_LINE.read_text())
    assert [s['action'] for s in recorded] == [s['action'] for s in queen['expected']]
    pinned = [s['digest_after'] for s in queen['expected']]
    rust_lane = [s['digest_after'] for s in recorded]
    assert pinned[:-1] == rust_lane[:-1]
    assert pinned[-1] != rust_lane[-1]


def test_real_queen_capture_replays_through_rust(queen, monkeypatch):
    """LOCAL-ONLY: needs a release `sts-sim`, which no CI lane builds (#3263).

    The testdata's checkpoints were trimmed to the fields compared before
    #3335 (energy, turn, piles), and its source capture is not in the local
    corpus, so the #3335 player fields (gold, stars, potion belt, orbs) are
    switched off here rather than backfilled with invented native values.
    They are witnessed by `test_issue3335_player_fields.py`.

    CI skips this; the binary-free test above keeps its pin tied to the gated
    Rust-lane line. Refresh `expected` only after deciding the move is an
    intended engine change (the `.mcr` checkpoints below must still pass), and
    name the merge that moved it: #2638 (step 34's hook-deactivation carrier)
    and #3088 (steps 7 and 27, the paused SetupPlayerTurn side-start tail).
    """
    binary = review.default_binary()
    if binary is None:
        pytest.skip('local-only: build versions/v0.111.0/rust/target/release/'
                    'sts-sim or set STS_SIM_EXACT_SOLVER (no CI lane builds it, #3263)')
    monkeypatch.setattr(replay, '_native_player_field_mismatch', lambda state, p: None)
    witness = replay.recorded_witness(binary, queen['entry'], queen['replay'], {'won': False, 'final_hp': 0})
    assert witness['actions'] == [s['action'] for s in queen['expected']]
    assert witness['native_checkpoints'] == 28
    line = review.replay_line(binary, queen['entry'], witness)
    actions = [a for g in line for a in g['actions']]
    assert [a['digest_after'] for a in actions] == [s['digest_after'] for s in queen['expected']]
    assert len(line) == 5 and len(actions) == 35
    assert all('state_after' in a for a in actions)
    assert actions[0]['cards'] == ['HOWL_FROM_BEYOND+']
    assert any(a.get('exhaust') == 'DEFEND_IRONCLAD' for a in actions)
    assert actions[-1]['state_after']['hp'] == 0
    damaged = copy.deepcopy(queen['replay'])
    damaged['checksums'][0]['full_state']['creatures'][0]['current_hp'] -= 1
    with pytest.raises(ValueError, match='native'):
        replay.recorded_witness(binary, queen['entry'], damaged, {'won': False, 'final_hp': 0})
    with pytest.raises(ValueError, match='recorded result'):
        replay.recorded_witness(binary, queen['entry'], queen['replay'], {'won': True, 'final_hp': 23})
    damaged = copy.deepcopy(queen['replay'])
    damaged['events'] = damaged['events'][:10]
    damaged['checksums'] = []
    with pytest.raises(ValueError, match='before combat completes'):
        replay.recorded_witness(binary, queen['entry'], damaged, {'won': False, 'final_hp': 0})


@pytest.mark.parametrize('player', [
    {'hp': 100, 'max_hp': 100, 'energy': 20, 'player_phase': 3,
     'next_card_uid': 2, 'lightning_rod': 2, 'orb_slots': 3},
    {'hp': 100, 'max_hp': 100, 'energy': 20, 'player_phase': 3,
     'genesis': 2, 'star_next_turn': 3,
     'star_energy_reset_order': ['star_next_turn', 'genesis']},
], ids=['legacy-lightning-rod-singleton', 'legacy-star-subgroup'])
def test_rust_replay_authenticates_legacy_inferred_reset_order_roots(player):
    """Hydrating a unique legacy order may not add a wire field on load."""
    binary = review.default_binary()
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


@pytest.mark.parametrize('floor', [42, 43, 46])
def test_retained_knights_lines_have_no_opaque_card_choices(floor):
    binary = review.default_binary()
    if binary is None:
        pytest.skip('set STS_SIM_EXACT_SOLVER to run replay integration')
    fixtures = Path(__file__).parents[1] / 'eval/search/knights-review-v1'
    entry = json.loads((fixtures / f'floor{floor}.canonical.json').read_text())
    saved = json.loads((fixtures / f'floor{floor}-solver-line.json').read_text())
    witness = dict(saved, final_digest=saved['digest'], combat_hp=saved['hp'], won=True)
    line = review.replay_line(binary, entry, witness)
    choices = [a for turn in line for a in turn['actions'] if a['kind'] == 'select']
    assert choices and all('selection_operation' in a for a in choices)
    assert all('choice' not in a for a in choices)
    if floor == 42:
        brand = next(a for a in choices if a['selection_source'] == 'BRAND')
        assert brand['cards'] == ['BLOODLETTING+']
        assert brand['selection_operation'] == 'exhaust'
