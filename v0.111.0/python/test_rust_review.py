"""The capture run, the entry reset witness and the state labels, without a local binary."""
import copy
import json
from types import SimpleNamespace

import pytest
import rust_replay
import rust_review as review


def replay_fixture(tmp_path, monkeypatch):
    """A synthetic capture and save projection for one fight at depth 32."""
    import live_coach
    import provenance_entry
    import review_provenance as provenance
    from sts2_rng import RunRngSet

    path = tmp_path / "run.json"
    path.write_text(json.dumps({"start_time": 123}))
    fight = SimpleNamespace(node_index=32, encounter_id="ENCOUNTER.THE_INSATIABLE_BOSS", node_type="boss")
    run = SimpleNamespace(build_id="v0.111.0", seed="SEED", fights=[fight],
                          character='CHARACTER.IRONCLAD')
    monkeypatch.setattr(review.relay_parser, "parse_run", lambda _: run)
    names = {**live_coach.STREAMS, 'combat_orbs': 'CombatOrbs'}
    rngs = RunRngSet('SEED', build='v0.111.0')
    native = {'seed': 'SEED', 'counters': {}, 'states': {}}
    for stream in names.values():
        rng = rngs[stream]._random
        native['counters'][stream] = 0
        native['states'][stream] = [rng.s0, rng.s1, rng.s2, rng.s3]
    projection = {'run_rng': {k: {'counter': 0} for k in names}, 'unlock_state': {}}
    unlocks = {'unlocked_epochs': ['IRONCLAD2_EPOCH'], 'encounters_seen': [], 'number_of_runs': 3}
    replay = {'version': 'v0.111.0', 'run': {
        'start_time': 123, 'rng': native,
        'players': [{'unlock_state': {'unlocked_epochs': [], 'encounters_seen': [],
                                      'number_of_runs': 0}}]}, 'events': []}
    monkeypatch.setattr(provenance, '_history_depth', lambda _: 32)
    monkeypatch.setattr(provenance, '_encounter_matches', lambda *args: True)
    monkeypatch.setattr(provenance_entry, 'provenance_entry_for_fight', lambda *a, **kw: None)
    monkeypatch.setattr(provenance_entry, '_complete_projected_player',
                        lambda _: {'unlock_state': unlocks})
    replay_fixture.unlocks = unlocks
    return path, replay, projection


def test_the_capture_run_is_the_embedded_run_with_the_saves_unlock_profile(tmp_path, monkeypatch):
    """#2972: `--capture-run` roots the capture's own embedded run, with only the
    unlock profile taken from the resolved save projection."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    run, fight, capture = review.replay_capture_run(path, 0, replay, projection)
    assert run.seed == 'SEED' and fight.encounter_id == 'ENCOUNTER.THE_INSATIABLE_BOSS'
    assert capture['players'][0]['unlock_state'] == replay_fixture.unlocks
    assert capture['rng'] == replay['run']['rng']
    assert capture['start_time'] == 123
    # The capture itself is not mutated.
    assert replay['run']['players'][0]['unlock_state']['number_of_runs'] == 0


def test_a_branch_admits_its_source_capture_only_below_the_branch_floor(tmp_path, monkeypatch):
    """#3373: the fixture's fight is at depth 32 of a run started at 123; the
    capture carries its SOURCE run's start_time, 50. The re-check here is
    review_provenance's association rule, not a second copy of it."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay['run']['start_time'] = 50
    with pytest.raises(ValueError, match='another run or fight'):
        review.replay_capture_run(path, 0, replay, projection)
    with pytest.raises(ValueError, match='another run or fight'):
        review.replay_capture_run(path, 0, replay, projection, ((50, 32),))
    review.replay_capture_run(path, 0, replay, projection, ((50, 33),))
    replay['run']['rng']['seed'] = 'OTHER'
    with pytest.raises(ValueError, match='another run or fight'):
        review.replay_capture_run(path, 0, replay, projection, ((50, 33),))


@pytest.mark.parametrize('damage,message', [
    ('seed', 'another run or fight'), ('time', 'another run or fight'),
    ('counter', 'does not match replay RNG counters'),
    ('words', 'disagree with seed/counters'),
    ('build', 'another build'), ('players', 'requires one player'),
    ('stream', 'malformed captured RNG stream'), ('index', 'out of range')])
def test_capture_run_checks_refuse_conflicting_evidence(tmp_path, monkeypatch, damage, message):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    index = 0
    if damage == 'seed':
        replay['run']['rng']['seed'] = 'OTHER'
    elif damage == 'time':
        replay['run']['start_time'] += 1
    elif damage == 'counter':
        projection['run_rng']['shuffle']['counter'] += 1
    elif damage == 'words':
        replay['run']['rng']['states']['Shuffle'][0] ^= 1
    elif damage == 'build':
        replay['version'] = 'v0.110.1'
    elif damage == 'players':
        replay['run']['players'].append(copy.deepcopy(replay['run']['players'][0]))
    elif damage == 'stream':
        replay['run']['rng']['states']['Shuffle'] = [1, 2, 3]
    else:
        index = 1
    with pytest.raises(ValueError, match=message):
        review.replay_capture_run(path, index, replay, projection)


def test_default_canonical_energy_is_three():
    assert review.snapshot({'player': {}, 'piles': {}})['energy'] == 3


def test_native_reset_power_list_keeps_serialized_order_and_rejects_duplicates():
    creature = {'powers': [
        {'id': 'SPINNER_POWER', 'amount': 1},
        {'id': 'GENESIS_POWER', 'amount': 2},
        {'id': 'STRENGTH_POWER', 'amount': 3},
        {'id': 'RADIANCE_POWER', 'amount': 1},
    ]}
    assert rust_replay.native_after_energy_reset_powers(creature) == [
        ('spinner', 1), ('genesis', 2), ('radiance', 1)]
    with pytest.raises(ValueError, match='malformed native'):
        rust_replay.native_after_energy_reset_powers(
            {'powers': [{'id': 'GENESIS_POWER', 'amount': 1},
                        {'id': 'GENESIS_POWER', 'amount': 2}]})
    with pytest.raises(ValueError, match='missing native'):
        rust_replay.native_after_energy_reset_powers({})
    with pytest.raises(ValueError, match='malformed native'):
        rust_replay.native_after_energy_reset_powers({'powers': None})


def _intent_doc(turn, hp, monsters):
    return {'player': {'turn': turn, 'hp': hp, 'block': 0, 'energy': 3},
            'piles': {'hand': [{'id': 'STRIKE_IRONCLAD', 'uid': 1}]},
            'monsters': monsters}


def test_snapshot_exports_rust_intents_and_falls_back_to_next_move():
    doc = _intent_doc(1, 70, [
        {'kind': 'THE_INSATIABLE', 'uid': 0, 'hp': 300, 'max_hp': 341},
        {'kind': 'EXOSKELETON', 'uid': 1, 'hp': 20, 'next_move': 'ENRAGE_MOVE'},
        {'kind': 'EXOSKELETON', 'uid': 2, 'hp': 20, 'next_move': 'SKITTER_MOVE'}])
    intents = [
        {'uid': 0, 'intent': 'THRASH_MOVE', 'intent_damage': 20, 'intent_hits': 2},
        {'uid': 1, 'intent': 'ENRAGE_MOVE'},
        {'uid': 2, 'intent': None}]
    enemies = review.snapshot(doc, intents)['enemies']
    assert enemies == [
        {'name': 'THE_INSATIABLE', 'hp': 300, 'maxSeen': 341, 'intent': 'THRASH_MOVE',
         'intent_damage': 20, 'intent_hits': 2},
        {'name': 'EXOSKELETON', 'hp': 20, 'maxSeen': 20, 'intent': 'ENRAGE_MOVE'},
        # The query named nothing: the projected random-table move stands, and
        # no damage is invented for it.
        {'name': 'EXOSKELETON', 'hp': 20, 'maxSeen': 20, 'intent': 'SKITTER_MOVE'}]
    # Without the query (an older binary) the snapshot keeps its old shape.
    assert [e['intent'] for e in review.snapshot(doc)['enemies']] == [
        None, 'ENRAGE_MOVE', 'SKITTER_MOVE']
    with pytest.raises(ValueError, match='monster roster'):
        review.snapshot(doc, intents[:2])
    with pytest.raises(ValueError, match='monster roster'):
        review.snapshot(doc, [dict(i, uid=9) for i in intents])


def test_card_name_carries_the_upgrade_and_the_enchantment():
    assert review.card_name({'id': 'BASH'}) == 'BASH'
    assert review.card_name({'id': 'BASH', 'upgrade': 1}) == 'BASH+'
    assert review.card_name({'id': 'BASH', 'enchantment': ['SHARP', 2]}) == 'BASH [SHARP 2]'


# The floor-save root's AfterEnergyReset order (`tools/live_coach.py`)
# ---------------------------------------------------------------------------

ROOT = {'player': {'hp': 66}}
WITNESSED = {'player': {'hp': 66, 'after_energy_reset_order': []}}


def test_a_root_the_boundary_loads_carries_the_empty_reset_order(monkeypatch):
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: None)
    root = copy.deepcopy(ROOT)
    assert review._with_entry_reset_witness('sts-sim', root) == WITNESSED
    assert root == ROOT


def test_a_floor_start_root_with_live_listeners_stays_legacy_unknown(monkeypatch):
    """The boundary rejects `[]` when reset listeners are live; the root is
    then left without an order, exactly as the Python adapter left it."""
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'after_energy_reset_order must exactly match '
         'live reset listeners'}))
    assert review._with_entry_reset_witness('sts-sim', copy.deepcopy(ROOT)) == ROOT


def test_an_unrelated_load_refusal_keeps_the_witness(monkeypatch):
    """Only an order refusal drops the witness; anything else is search's to report."""
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'entry needs argument shape at X'}))
    assert review._with_entry_reset_witness('sts-sim', copy.deepcopy(ROOT)) == WITNESSED


@pytest.mark.parametrize('answer,expected', [
    ({'ok': {'loaded': True}}, None),
    ({'refusal': {'site': 'load', 'kind': 'not_admitted', 'detail': 'x'}},
     json.dumps({'site': 'load', 'kind': 'not_admitted', 'detail': 'x'})),
])
def test_load_refusal_reads_the_line_after_the_banner(monkeypatch, answer, expected):
    seen = {}

    def run(argv, **kw):
        seen['argv'], seen['input'] = argv, kw['input']
        return SimpleNamespace(returncode=0, stderr='', stdout=json.dumps(
            {'protocol': 'diff-serve'}) + '\n' + json.dumps(answer) + '\n' + json.dumps({'ok': {'quit': True}}) + '\n')
    monkeypatch.setattr(review.subprocess, 'run', run)
    assert review._load_refusal('sts-sim', {'player': {}}) == expected
    assert seen['argv'] == ['sts-sim', 'diff-serve']
    assert json.loads(seen['input'].splitlines()[0]) == {'cmd': 'load', 'entry': {'player': {}}}


def test_load_refusal_without_an_answer_line_fails_closed(monkeypatch):
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: SimpleNamespace(
        returncode=1, stderr='panic', stdout=json.dumps({'protocol': 'diff-serve'}) + '\n'))
    with pytest.raises(ValueError, match='diff-serve answered no load line'):
        review._load_refusal('sts-sim', {'player': {}})
