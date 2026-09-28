"""Snapshot identity and Rust-only publication contracts, without a local binary."""
import copy
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
import rust_review as review


def fixture(tmp_path, monkeypatch):
    path = tmp_path / "run.json"
    path.write_text(json.dumps({"start_time": 123}))
    fight = SimpleNamespace(node_index=32, encounter_id="ENCOUNTER.THE_INSATIABLE_BOSS", node_type="boss")
    run = SimpleNamespace(build_id="v0.111.0", seed="SEED", fights=[fight])
    monkeypatch.setattr(review.relay_parser, "parse_run", lambda _: run)
    monkeypatch.setattr(review.live_coach, "build_entry", lambda *args: {"node_index": 32})
    # The Rust opening is the root builder (#2827); the fixture stands in for
    # `sts-sim entry --opening` and for the boundary's load answer.
    monkeypatch.setattr(review, "_rust_opening", lambda binary, save, *args: copy.deepcopy(save["root"]))
    monkeypatch.setattr(review, "_load_refusal", lambda binary, document: None)
    saves = [{"id": "one", "game_build": "v0.111.0", "save": {
        "rng": {"seed": "SEED"}, "start_time": 123, "root": {"player": {"hp": 66}}}}]
    return path, saves


WITNESSED = {"player": {"hp": 66, "after_energy_reset_order": []}}


def test_equivalent_snapshots_resolve_but_conflicting_roots_refuse(tmp_path, monkeypatch):
    path, saves = fixture(tmp_path, monkeypatch)
    saves.append(copy.deepcopy(saves[0]))
    assert review.build_root(path, 0, saves, "sts-sim")[2] == WITNESSED
    saves[1]["save"]["root"]["player"]["hp"] = 65
    with pytest.raises(ValueError, match="snapshots disagree"):
        review.build_root(path, 0, saves, "sts-sim")


@pytest.mark.parametrize("field,value", [("start_time", 124), ("rng", {"seed": "OTHER"})])
def test_snapshot_run_identity_never_borrows_a_different_capture(tmp_path, monkeypatch, field, value):
    path, saves = fixture(tmp_path, monkeypatch)
    saves[0]["save"][field] = value
    with pytest.raises(ValueError, match="another run"):
        review.build_root(path, 0, saves, "sts-sim")


def test_search_refusal_never_publishes_a_partial_portfolio(tmp_path, monkeypatch):
    from review_summary import ReviewConfig
    path, saves = fixture(tmp_path, monkeypatch)
    saves[0]['sha256'] = 'fixture'
    results = iter([
        SimpleNamespace(returncode=0, stdout=json.dumps({
            'entry_digest': review.canonical_document.differential_digest(WITNESSED),
            'best': {'won': True, 'combat_hp': 55, 'turn': 11, 'actions': [], 'final_digest': 'end'}})),
        SimpleNamespace(returncode=2, stderr='unsupported transition'),
    ])
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: next(results))
    monkeypatch.setattr(review, 'replay_line', lambda *a: [{'turn': 1, 'actions': []}])
    monkeypatch.setattr(review.review, 'simulator_document', lambda *a: {})
    document = review.generate(path, 0, saves, ReviewConfig(rust_exact_solver_binary=tmp_path / 'solver'))
    assert document['status'] == 'refused'
    assert 'unsupported transition' in document['refusal']['message']
    assert 'best_actual_seed' not in document


def test_search_without_a_witness_names_its_first_pruned_refusal(tmp_path, monkeypatch):
    """#2949: refused branches are pruned in Rust; with no line left, say why."""
    from review_summary import ReviewConfig
    path, saves = fixture(tmp_path, monkeypatch)
    digest = review.canonical_document.differential_digest(WITNESSED)
    refusal = {'status': 'refused', 'detail': 'MalformedArgs("Thrash damage candidate")',
               'actions': [{'kind': 'end'}]}
    results = iter([
        SimpleNamespace(returncode=0, stdout=json.dumps({
            'entry_digest': digest, 'best': None, 'refusals': 3, 'first_refusal': refusal})),
        SimpleNamespace(returncode=0, stdout=json.dumps({
            'entry_digest': digest, 'best': None, 'refusals': 0, 'first_refusal': None})),
    ])
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: next(results))
    monkeypatch.setattr(review.review, 'simulator_document', lambda *a: {})
    document = review.generate(path, 0, saves, ReviewConfig(rust_exact_solver_binary=tmp_path / 'solver'))
    assert document['status'] == 'refused'
    assert 'Thrash damage candidate' in document['refusal']['message']


def test_search_root_mismatch_never_reaches_replay(tmp_path, monkeypatch):
    from review_summary import ReviewConfig
    path, saves = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: SimpleNamespace(
        returncode=0, stdout=json.dumps({'entry_digest': 'another-root'})))
    monkeypatch.setattr(review.review, 'simulator_document', lambda *a: {})
    document = review.generate(path, 0, saves, ReviewConfig(rust_exact_solver_binary=tmp_path / 'solver'))
    assert document['status'] == 'refused'
    assert 'root mismatch' in document['refusal']['message']


def replay_fixture(tmp_path, monkeypatch):
    """Synthetic entry; no action suffix is needed to authenticate its root."""
    import live_coach
    import mcr_native
    import review_provenance as provenance
    import review_summary_v2 as adapter
    from sts2_rng import RunRngSet

    path, _ = fixture(tmp_path, monkeypatch)
    run = review.relay_parser.parse_run(path)
    run.character = 'CHARACTER.IRONCLAD'
    names = {**live_coach.STREAMS, 'combat_orbs': 'CombatOrbs'}
    rngs = RunRngSet('SEED', build='v0.111.0')
    native = {'seed': 'SEED', 'counters': {}, 'states': {}}
    for attr, stream in names.items():
        rng = rngs[stream]._random
        native['counters'][stream] = 0
        native['states'][stream] = [rng.s0, rng.s1, rng.s2, rng.s3]
    projection = {'run_rng': {k: {'counter': 0} for k in names}, 'unlock_state': {}}
    unlocks = {'unlocked_epochs': ['IRONCLAD2_EPOCH'], 'encounters_seen': [], 'number_of_runs': 3}
    replay = {'version': 'v0.111.0', 'run': {
        'start_time': 123, 'rng': native,
        'players': [{'unlock_state': {'unlocked_epochs': [], 'encounters_seen': [],
                                      'number_of_runs': 0}}]}, 'events': [],
        'checksums': [{'context': 'After player turn start', 'full_state': {
            'rng': copy.deepcopy(native), 'players': [{'energy': 3, 'turn_number': 1,
                'gold': 0, 'stars': 0, 'max_potion_count': 0, 'potions': [], 'orbs': [],
                'piles': [{'pile_type': k, 'cards': []} for k in ('Hand','Draw','Discard','Exhaust','Play')]}],
            'creatures': [{'player_id': 1, 'current_hp': 66, 'max_hp': 80,
                           'block': 0, 'powers': []}]}}]}
    # The Rust opening as `sts-sim entry --capture-run --opening` answers it:
    # canonical-v2, with the nine combat streams the checkpoint carries.
    opening = {'schema': 'sts-sim-canonical-v2', 'player': {'hp': 66, 'max_hp': 80},
               'monsters': [], 'piles': {},
               'rng': {attr: {'words': list(native['states'][stream]), 'counter': 0}
                       for attr, stream in mcr_native._REPRESENTED_RNG_STREAMS.items()}}
    calls = []

    def rust_opening(binary, run, fight, build, *, source='--save', native_checkpoints=False):
        calls.append({'binary': binary, 'run': copy.deepcopy(run), 'source': source,
                      'encounter': fight.encounter_id, 'build': build,
                      'native_checkpoints': native_checkpoints})
        document = copy.deepcopy(opening)
        if native_checkpoints:
            return document, copy.deepcopy(replay_fixture.recorded)
        return document
    monkeypatch.setattr(provenance, '_history_depth', lambda _: 32)
    monkeypatch.setattr(provenance, '_encounter_matches', lambda *args: True)
    monkeypatch.setattr(adapter, 'provenance_entry_for_fight', lambda *a, **kw: None)
    monkeypatch.setattr(adapter, '_complete_projected_player', lambda _: {'unlock_state': unlocks})
    monkeypatch.setattr(review, '_rust_opening', rust_opening)
    monkeypatch.setattr(review, 'default_binary', lambda: 'sts-sim')
    replay_fixture.calls, replay_fixture.opening, replay_fixture.unlocks = calls, opening, unlocks
    replay_fixture.recorded = None
    return path, replay, projection


def test_empty_action_suffix_does_not_prevent_captured_root_admission(tmp_path, monkeypatch):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    root = review.build_replay_root(path, 0, replay, projection)[2]
    assert root == dict(replay_fixture.opening, player=dict(
        replay_fixture.opening['player'], after_energy_reset_order=[]))


def test_the_replay_root_is_the_rust_opening_of_the_capture_run(tmp_path, monkeypatch):
    """#2972: `--capture-run` on the capture's own embedded run, with only the
    unlock profile taken from the resolved save projection."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    review.build_replay_root(path, 0, replay, projection, 'bin/sts-sim')
    [call] = replay_fixture.calls
    assert call['source'] == '--capture-run'
    assert (call['binary'], call['build']) == ('bin/sts-sim', 'v0.111.0')
    assert call['encounter'] == 'ENCOUNTER.THE_INSATIABLE_BOSS'
    assert call['run']['players'][0]['unlock_state'] == replay_fixture.unlocks
    assert call['run']['rng'] == replay['run']['rng']
    # The capture itself is not mutated.
    assert replay['run']['players'][0]['unlock_state']['number_of_runs'] == 0


def test_the_native_reset_order_is_carried_onto_the_replay_root(tmp_path, monkeypatch):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay['checksums'][0]['full_state']['creatures'][0]['powers'] = [
        {'id': 'SPINNER_POWER', 'amount': 1}, {'id': 'STRENGTH_POWER', 'amount': 2},
        {'id': 'GENESIS_POWER', 'amount': 2}]
    replay_fixture.opening['player'].update(spinner=1, genesis=2)
    loaded = []
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: loaded.append(
        document['player']['after_energy_reset_order']))
    root = review.build_replay_root(path, 0, replay, projection)[2]
    assert root['player']['after_energy_reset_order'] == ['spinner', 'genesis']
    assert loaded == [['spinner', 'genesis']]


@pytest.mark.parametrize('modeled', [
    {'spinner': 1},                                # a native listener missing
    {'spinner': 1, 'genesis': 3},                  # a different amount
    {'spinner': 1, 'genesis': 2, 'radiance': 1},   # a listener native lacks
])
def test_opening_reset_listeners_are_checked_against_the_checkpoint(tmp_path, monkeypatch, modeled):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay['checksums'][0]['full_state']['creatures'][0]['powers'] = [
        {'id': 'SPINNER_POWER', 'amount': 1}, {'id': 'GENESIS_POWER', 'amount': 2}]
    replay_fixture.opening['player'].update(modeled)
    with pytest.raises(ValueError, match='reset listeners differ'):
        review.build_replay_root(path, 0, replay, projection)


def test_a_boundary_rejection_of_the_native_order_is_a_refusal(tmp_path, monkeypatch):
    """The order is observed on this path, never proposed, so it does not
    fall back to legacy-unknown the way the floor-snapshot witness does."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'after_energy_reset_order must exactly match '
         'live reset listeners'}))
    with pytest.raises(ValueError, match='rejected the native AfterEnergyReset order'):
        review.build_replay_root(path, 0, replay, projection)


def test_the_opening_checkpoint_is_compared_against_the_pre_autopre_state(tmp_path, monkeypatch):
    """#3414, the production side of #3392: native writes the opening
    checkpoint before turn one's AutoPre, where Whispering Earring's loop
    spends energy. The recorded pre-AutoPre state is compared; the root is
    what search loads."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    before = copy.deepcopy(replay_fixture.opening)
    replay_fixture.opening['player'].update(energy=1)
    with pytest.raises(ValueError, match='differs from native player state'):
        review.build_replay_root(path, 0, replay, projection)
    replay_fixture.recorded = [{'kind': 'after_player_turn_start', 'state': before}]
    root = review.build_replay_root(path, 0, replay, projection)[2]
    assert root['player']['energy'] == 1
    assert replay_fixture.calls[-1]['native_checkpoints'] is True


def test_a_listener_the_autopre_loop_acquired_follows_the_native_order(tmp_path, monkeypatch):
    """A reset listener acquired after the checkpoint joins `Creature.Powers`
    after every checkpoint power, in the root's own acquisition order."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay['checksums'][0]['full_state']['creatures'][0]['powers'] = [
        {'id': 'SPINNER_POWER', 'amount': 1}]
    replay_fixture.opening['player'].update(spinner=1)
    before = copy.deepcopy(replay_fixture.opening)
    replay_fixture.opening['player'].update(
        genesis=2, after_energy_reset_order=['genesis', 'spinner'])
    replay_fixture.recorded = [{'kind': 'after_player_turn_start', 'state': before}]
    root = review.build_replay_root(path, 0, replay, projection)[2]
    assert root['player']['after_energy_reset_order'] == ['spinner', 'genesis']


@pytest.mark.parametrize('recorded', [
    {'kind': 'after_player_turn_start'},
    [{'kind': 'after_hand_draw', 'state': {}}],
    [{'kind': 'after_player_turn_start', 'state': None}],
])
def test_a_malformed_opening_checkpoint_report_is_a_refusal(tmp_path, monkeypatch, recorded):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay_fixture.recorded = recorded
    with pytest.raises(ValueError, match='opening'):
        review.build_replay_root(path, 0, replay, projection)


def test_rust_opening_unwraps_its_native_checkpoints(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='monster')
    seen = {}
    root = {'schema': 'sts-sim-canonical-v2', 'player': {}}

    def run(argv, **kw):
        seen['argv'] = argv
        return SimpleNamespace(returncode=0, stderr='', stdout=json.dumps({
            'schema': 'sts-sim-opening-checkpoints-v1', 'state': root,
            'native_checkpoints': [{'kind': 'after_player_turn_start', 'state': root}]}))
    monkeypatch.setattr(review.subprocess, 'run', run)
    document, recorded = review._rust_opening(
        'sts-sim', {}, fight, 'v0.111.0', source='--capture-run', native_checkpoints=True)
    assert document == root and recorded[0]['kind'] == 'after_player_turn_start'
    assert seen['argv'][-2:] == ['--opening', '--native-checkpoints']


def test_an_unrelated_load_refusal_leaves_the_replay_root_to_search(tmp_path, monkeypatch):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'entry needs argument shape at X'}))
    root = review.build_replay_root(path, 0, replay, projection)[2]
    assert root['player']['after_energy_reset_order'] == []


@pytest.mark.parametrize('damage', ['none', 'context', 'no_state'])
def test_a_capture_without_its_opening_checkpoint_refuses(tmp_path, monkeypatch, damage):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    if damage == 'none':
        replay['checksums'] = []
    elif damage == 'context':
        replay['checksums'][0]['context'] = 'finished action execution PlayCardAction'
    else:
        replay['checksums'][0]['full_state'] = None
    with pytest.raises(ValueError, match='no opening checkpoint'):
        review.build_replay_root(path, 0, replay, projection)
    assert replay_fixture.calls == []


def test_the_replay_root_needs_a_rust_binary(tmp_path, monkeypatch):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review, 'default_binary', lambda: None)
    with pytest.raises(ValueError, match='Rust solver executable is unavailable'):
        review.build_replay_root(path, 0, replay, projection)


def test_a_branch_admits_its_source_capture_only_below_the_branch_floor(tmp_path, monkeypatch):
    """#3373: the fixture's fight is at depth 32 of a run started at 123; the
    capture carries its SOURCE run's start_time, 50. The re-check here is
    review_provenance's association rule, not a second copy of it."""
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    replay['run']['start_time'] = 50
    with pytest.raises(ValueError, match='another run or fight'):
        review.build_replay_root(path, 0, replay, projection)
    with pytest.raises(ValueError, match='another run or fight'):
        review.build_replay_root(path, 0, replay, projection, None, ((50, 32),))
    review.build_replay_root(path, 0, replay, projection, None, ((50, 33),))
    replay['run']['rng']['seed'] = 'OTHER'
    with pytest.raises(ValueError, match='another run or fight'):
        review.build_replay_root(path, 0, replay, projection, None, ((50, 33),))


def test_opening_enchantments_are_checked_against_the_checkpoint(tmp_path, monkeypatch):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    hand = replay['checksums'][0]['full_state']['players'][0]['piles'][0]
    hand['cards'] = [{'card': {'id': 'BASH', 'upgrade_level': 0,
                               'enchantment': {'id': 'SHARP', 'level': 2}}}]
    replay_fixture.opening['piles'] = {'hand': [{'id': 'BASH', 'uid': 1, 'enchantment': ['SHARP', 2]}]}
    review.build_replay_root(path, 0, replay, projection)
    replay_fixture.opening['piles']['hand'][0]['enchantment'] = ['SHARP', 1]
    with pytest.raises(ValueError, match='enchantments differ'):
        review.build_replay_root(path, 0, replay, projection)
    del replay_fixture.opening['piles']['hand'][0]['enchantment']
    with pytest.raises(ValueError, match='enchantments differ'):
        review.build_replay_root(path, 0, replay, projection)


@pytest.mark.parametrize('damage', ['seed', 'time', 'counter', 'words', 'hp', 'piles', 'rng', 'monster'])
def test_replay_entry_checks_refuse_conflicting_evidence(tmp_path, monkeypatch, damage):
    path, replay, projection = replay_fixture(tmp_path, monkeypatch)
    if damage == 'seed':
        replay['run']['rng']['seed'] = 'OTHER'
    elif damage == 'time':
        replay['run']['start_time'] += 1
    elif damage == 'counter':
        projection['run_rng']['shuffle']['counter'] += 1
    elif damage == 'words':
        replay['run']['rng']['states']['Shuffle'][0] ^= 1
    elif damage == 'hp':
        replay['checksums'][0]['full_state']['creatures'][0]['current_hp'] -= 1
    elif damage == 'rng':
        replay['checksums'][0]['full_state']['rng']['counters']['Niche'] += 1
    elif damage == 'monster':
        replay['checksums'][0]['full_state']['creatures'].append(
            {'monster_id': 'NIBBIT', 'player_id': None, 'current_hp': 40, 'max_hp': 40,
             'block': 0, 'powers': []})
    else:
        replay['checksums'][0]['full_state']['players'][0]['piles'][0]['cards'] = [
            {'card': {'id': 'STRIKE_IRONCLAD', 'upgrade_level': 0}, 'affliction': None,
             'energy_cost': None, 'keywords': None}]
    with pytest.raises(ValueError):
        review.build_replay_root(path, 0, replay, projection)


def test_default_canonical_energy_is_three():
    assert review.snapshot({'player': {}, 'piles': {}})['energy'] == 3


def test_native_reset_power_list_keeps_serialized_order_and_rejects_duplicates():
    creature = {'powers': [
        {'id': 'SPINNER_POWER', 'amount': 1},
        {'id': 'GENESIS_POWER', 'amount': 2},
        {'id': 'STRENGTH_POWER', 'amount': 3},
        {'id': 'RADIANCE_POWER', 'amount': 1},
    ]}
    assert review._native_after_energy_reset_powers(creature) == [
        ('spinner', 1), ('genesis', 2), ('radiance', 1)]
    with pytest.raises(ValueError, match='malformed native'):
        review._native_after_energy_reset_powers(
            {'powers': [{'id': 'GENESIS_POWER', 'amount': 1},
                        {'id': 'GENESIS_POWER', 'amount': 2}]})
    with pytest.raises(ValueError, match='missing native'):
        review._native_after_energy_reset_powers({})
    with pytest.raises(ValueError, match='malformed native'):
        review._native_after_energy_reset_powers({'powers': None})


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


class _IntentReplay:
    """A diff-serve stand-in: two turns, one action each, intents per state."""

    def __init__(self, known=True):
        self.known = known
        self.states = [
            _intent_doc(1, 70, [{'kind': 'THE_INSATIABLE', 'uid': 0, 'hp': 300}]),
            _intent_doc(2, 52, [{'kind': 'THE_INSATIABLE', 'uid': 0, 'hp': 294}]),
            _intent_doc(2, 52, [{'kind': 'THE_INSATIABLE', 'uid': 0, 'hp': 288}])]
        self.intents = [
            [{'uid': 0, 'intent': 'THRASH_MOVE', 'intent_damage': 9, 'intent_hits': 2}],
            [{'uid': 0, 'intent': 'LUNGING_BITE_MOVE', 'intent_damage': 31, 'intent_hits': 1}],
            [{'uid': 0, 'intent': 'LUNGING_BITE_MOVE', 'intent_damage': 23, 'intent_hits': 1}]]
        self.at = 0

    def __call__(self, binary, entry):
        return self

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def ask(self, request):
        if request['cmd'] == 'intents':
            if not self.known:
                raise ValueError("Rust replay refused: {'refusal': {'kind': 'unknown_command'}}")
            return {'intents': self.intents[self.at]}
        if request['cmd'] == 'apply':
            self.at += 1
            return {'digest': f'd{self.at}'}
        assert request['cmd'] == 'project'
        return {'state': self.states[self.at]}


@pytest.mark.parametrize('known', [True, False])
def test_replay_line_groups_carry_turn_opening_enemies(monkeypatch, known):
    fake = _IntentReplay(known)
    monkeypatch.setattr(review, 'RustReplay', fake)
    monkeypatch.setattr(review.canonical_document, 'differential_digest', lambda state: 'final')
    best = {'actions': [{'kind': 'end'}, {'kind': 'play', 'uid': 1, 'target': 0}],
            'final_digest': 'final', 'won': False, 'combat_hp': 52}
    groups = review.replay_line('sim', fake.states[0], best)
    assert [g['turn'] for g in groups] == [1, 2]
    opening = [g['enemies'][0] for g in groups]
    after = [a['state_after']['enemies'][0] for g in groups for a in g['actions']]
    if known:
        # Each group's roster is the one the turn opened with, before its
        # first action; each action's snapshot has the intent after it.
        assert [(e['intent'], e['intent_damage'], e['intent_hits']) for e in opening] == [
            ('THRASH_MOVE', 9, 2), ('LUNGING_BITE_MOVE', 31, 1)]
        assert [(e['hp'], e['intent_damage']) for e in after] == [(294, 31), (288, 23)]
    else:
        assert [e['intent'] for e in opening + after] == [None] * 4
        assert not any('intent_damage' in e for e in opening + after)


# ---------------------------------------------------------------------------
# #2827: the Rust opening is the root builder
# ---------------------------------------------------------------------------

def test_a_floor_start_root_with_live_listeners_stays_legacy_unknown(tmp_path, monkeypatch):
    """The boundary rejects `[]` when reset listeners are live; the root is
    then left without an order, exactly as the Python adapter left it."""
    path, saves = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'after_energy_reset_order must exactly match '
         'live reset listeners'}))
    assert review.build_root(path, 0, saves, 'sts-sim')[2] == {'player': {'hp': 66}}


def test_an_unrelated_load_refusal_keeps_the_witness(tmp_path, monkeypatch):
    """Only an order refusal drops the witness; anything else is search's to report."""
    path, saves = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(review, '_load_refusal', lambda binary, document: json.dumps(
        {'kind': 'not_admitted', 'detail': 'entry needs argument shape at X'}))
    assert review.build_root(path, 0, saves, 'sts-sim')[2] == WITNESSED


def test_rust_opening_names_an_opening_refusal(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='monster')
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: SimpleNamespace(
        returncode=0, stderr='', stdout=json.dumps({
            'schema': 'sts-sim-entry-v1',
            'opening': {'built': False, 'refusal_class': 'stone_cracker_pre_draw_upgrade',
                        'detail': 'upgrades two cards'}})))
    with pytest.raises(ValueError, match='Rust opening refused: stone_cracker_pre_draw_upgrade'):
        review._rust_opening('sts-sim', {}, fight, 'v0.111.0')


def test_rust_opening_invokes_the_entry_opening_subcommand(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='elite')
    seen = {}

    def run(argv, **kw):
        seen['argv'] = argv
        return SimpleNamespace(returncode=0, stderr='', stdout=json.dumps(
            {'schema': 'sts-sim-canonical-v2', 'player': {}}))
    monkeypatch.setattr(review.subprocess, 'run', run)
    assert review._rust_opening('sts-sim', {}, fight, 'v0.111.0') == {
        'schema': 'sts-sim-canonical-v2', 'player': {}}
    argv = seen['argv']
    assert argv[:4] == ['sts-sim', 'entry', '--build', 'v0.111.0']
    assert argv[argv.index('--encounter') + 1] == 'ENCOUNTER.X'
    assert argv[argv.index('--node-type') + 1] == 'elite'
    assert argv[-1] == '--opening'


def test_rust_opening_reads_a_capture_run_through_its_own_flag(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='monster')
    seen = {}

    def run(argv, **kw):
        seen['argv'] = argv
        seen['input'] = json.loads(Path(argv[argv.index('--capture-run') + 1]).read_text())
        return SimpleNamespace(returncode=0, stderr='', stdout=json.dumps(
            {'schema': 'sts-sim-canonical-v2', 'player': {}}))
    monkeypatch.setattr(review.subprocess, 'run', run)
    review._rust_opening('sts-sim', {'acts': []}, fight, 'v0.111.0', source='--capture-run')
    assert '--save' not in seen['argv']
    assert seen['input'] == {'acts': []}
    assert seen['argv'][-1] == '--opening'


def test_rust_opening_refuses_an_unknown_input_flag(monkeypatch):
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: pytest.fail('ran'))
    with pytest.raises(ValueError, match='unknown Rust entry input'):
        review._rust_opening('sts-sim', {}, SimpleNamespace(), 'v0.111.0', source='--mcr')


def test_rust_opening_names_an_entry_refusal(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='monster')
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: SimpleNamespace(
        returncode=0, stderr='', stdout=json.dumps({
            'schema': 'sts-sim-entry-v1', 'refusal_class': 'unknown_save_field',
            'refusal': {'detail': 'unknown field `elites_visited`'}})))
    with pytest.raises(ValueError, match='Rust opening refused: unknown_save_field: unknown field'):
        review._rust_opening('sts-sim', {}, fight, 'v0.111.0')


def test_rust_opening_names_an_argv_failure(monkeypatch):
    fight = SimpleNamespace(encounter_id='ENCOUNTER.X', node_type='monster')
    monkeypatch.setattr(review.subprocess, 'run', lambda *a, **kw: SimpleNamespace(
        returncode=2, stderr='refusal: entry: unknown flag', stdout=''))
    with pytest.raises(ValueError, match='Rust opening CLI failed: refusal: entry: unknown flag'):
        review._rust_opening('sts-sim', {}, fight, 'v0.111.0')


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


def _two_searches(monkeypatch, uct, random):
    """Stand in for the UCT then random `sts-sim search` runs."""
    results = iter([{"entry_digest": "d", "best": uct}, {"entry_digest": "d", "best": random}])
    monkeypatch.setattr(review.subprocess, "run", lambda *a, **k: SimpleNamespace(
        returncode=0, stdout=json.dumps(next(results))))
    monkeypatch.setattr(review.canonical_document, "differential_digest", lambda _entry: "d")
    monkeypatch.setattr(review, "replay_line", lambda *a, **k: [{"turn": 1, "actions": []}])
    config = SimpleNamespace(actual_deadline=1.0, horizon=20)
    return review.search_outcomes("sts-sim", {"player": {"hp": 70}}, config)[0]


def _best(hp, digest, **extra):
    return {"actions": [], "combat_hp": hp, "turn": 3, "won": True, "final_digest": digest, **extra}


@pytest.mark.parametrize("kill_hp", [70, 20])
def test_a_battleworn_kill_outranks_a_timeout_at_any_hp(monkeypatch, kill_hp):
    # #3369: the timer's escape is a won combat that forfeits the event's
    # reward, so a kill ranks first at equal HP and at lower HP.
    outcomes = _two_searches(monkeypatch, _best(70, "timeout", battleworn_timeout=True),
                             _best(kill_hp, "kill", battleworn_timeout=False))
    assert [o["line_id"] for o in outcomes] == ["random", "uct"]
    assert all("battleworn_timeout" not in o for o in outcomes), "document schema unchanged"


def test_a_timeout_still_outranks_a_loss_and_other_fights_rank_by_hp(monkeypatch):
    lost = dict(_best(0, "lost"), won=False)
    outcomes = _two_searches(monkeypatch, lost, _best(5, "timeout", battleworn_timeout=True))
    assert [o["line_id"] for o in outcomes] == ["random", "uct"]
    # A search output without the flag (any other fight) keeps the HP order.
    outcomes = _two_searches(monkeypatch, _best(40, "a"), _best(50, "b"))
    assert [o["line_id"] for o in outcomes] == ["random", "uct"]
    outcomes = _two_searches(monkeypatch, _best(60, "a"), _best(50, "b"))
    assert [o["line_id"] for o in outcomes] == ["uct", "random"]


def _holdback_config(**kw):
    return SimpleNamespace(actual_deadline=1.0, horizon=20, **kw)


BELT = {"player": {"hp": 70, "potion_slots": ["FIRE_POTION", None, "FAIRY_IN_A_BOTTLE", "SKILL_POTION"]}}


def test_potion_holdback_defaults_to_none_and_counts_only_drinkable_potions():
    assert review.potion_holdback(BELT, _holdback_config()) is None
    assert review.potion_holdback(BELT, _holdback_config(potion_hold_count=0, potion_hold_slots=())) is None
    # Fairy in a Bottle is passive: two drinkable potions, not three.
    hold = review.potion_holdback(BELT, _holdback_config(potion_hold_count=1, potion_hold_slots=(3,)))
    assert hold == {"hold_count": 1, "hold_slots": [3], "held_potions": ["SKILL_POTION"], "max_drinks": 1}
    assert review.potion_holdback_argv(hold) == ["--max-potions", "1", "--hold-slot", "3"]
    slots_only = review.potion_holdback(BELT, _holdback_config(potion_hold_count=0, potion_hold_slots=(0,)))
    assert review.potion_holdback_argv(slots_only) == ["--hold-slot", "0"]
    hold_all = review.potion_holdback(BELT, _holdback_config(potion_hold_count=2, potion_hold_slots=()))
    assert review.potion_holdback_argv(hold_all) == ["--max-potions", "0"]


@pytest.mark.parametrize("count,slots,message", [
    (3, (), "cannot keep 3 of 2"),
    (0, (1,), "slot 1 holds no drinkable"),
    (0, (2,), "slot 2 holds no drinkable"),
    (0, (9,), "slot 9 holds no drinkable"),
])
def test_a_holdback_the_belt_cannot_honour_refuses(count, slots, message):
    with pytest.raises(ValueError, match=message):
        review.potion_holdback(BELT, _holdback_config(potion_hold_count=count, potion_hold_slots=slots))


def test_search_outcomes_passes_the_holdback_to_both_searches(monkeypatch):
    argvs = []

    def run(argv, **kw):
        argvs.append(argv)
        return SimpleNamespace(returncode=0, stdout=json.dumps({"entry_digest": "d", "best": None}))
    monkeypatch.setattr(review.subprocess, "run", run)
    monkeypatch.setattr(review.canonical_document, "differential_digest", lambda _entry: "d")
    review.search_outcomes("sts-sim", BELT, _holdback_config(potion_hold_count=2, potion_hold_slots=(0,)))
    assert [argv[3] for argv in argvs] == ["uct", "random"]
    assert all(argv[10:] == ["--max-potions", "0", "--hold-slot", "0"] for argv in argvs)
    argvs.clear()
    # The default search argv is unchanged.
    review.search_outcomes("sts-sim", BELT, _holdback_config())
    assert all(len(argv) == 10 for argv in argvs)


def test_record_potion_belt_names_the_entry_belt_and_any_holdback():
    document = {"metadata": {}}
    review.record_potion_belt(document, BELT, _holdback_config())
    assert document["metadata"] == {"entry_potion_slots": BELT["player"]["potion_slots"]}
    review.record_potion_belt(document, BELT, _holdback_config(potion_hold_count=1, potion_hold_slots=()))
    assert document["metadata"]["potion_holdback"]["max_drinks"] == 1
    empty = {"metadata": {}}
    review.record_potion_belt(empty, {"player": {"hp": 5}}, _holdback_config())
    assert empty["metadata"] == {"entry_potion_slots": []}


# ---------------------------------------------------------------------------
# Automatic plays (Hellraiser) are rows of their own, off `actions`
# ---------------------------------------------------------------------------

def _mark(kind, uid, card_id, turn, hp, block, hand, monster_hp):
    return {'kind': kind, 'source': 'Hellraiser',
            'card': {'uid': uid, 'id': card_id, 'upgrade': 0, 'enchantment': None},
            'state': {'turn': turn, 'hp': hp, 'block': block, 'energy': 3,
                      'hand': [{'uid': u, 'id': i, 'upgrade': 0, 'enchantment': None} for u, i in hand],
                      'monsters': [{'uid': 0, 'kind': 'ENTOMANCER', 'hp': monster_hp, 'max_hp': 165}]}}


class _AutoplayReplay(_IntentReplay):
    """End turn on turn 2 whose turn-3 draw feeds Hellraiser one Strike, then
    a turn-3 play that feeds it another mid-turn."""

    def __init__(self, rest_visible=False):
        super().__init__(known=True)
        monster = lambda hp: [{'kind': 'ENTOMANCER', 'uid': 0, 'hp': hp, 'max_hp': 165}]
        self.states = [
            _intent_doc(2, 22, monster(96)),
            dict(_intent_doc(3, 17, monster(70 if rest_visible else 76)),
                 piles={'hand': [{'id': 'BOLAS', 'uid': 5}, {'id': 'POMMEL_STRIKE', 'uid': 6}]}),
            dict(_intent_doc(3, 17, monster(52)), piles={'hand': [{'id': 'BOLAS', 'uid': 5}]}),
        ]
        self.intents = [[{'uid': 0, 'intent': 'SWARM_MOVE'}],
                        [{'uid': 0, 'intent': 'PHEROMONE_SPIT_MOVE'}],
                        [{'uid': 0, 'intent': 'PHEROMONE_SPIT_MOVE'}]]
        self.marks = [
            # A throwaway probe never reaches the producer: Rust keeps only
            # the committed marks (`engine::presentation::record`).
            [_mark('autoplay_begin', 9, 'STRIKE_IRONCLAD', 3, 17, 0, [(9, 'STRIKE_IRONCLAD')], 96),
             _mark('autoplay_end', 9, 'STRIKE_IRONCLAD', 3, 17, 0, [], 76)],
            [_mark('autoplay_begin', 7, 'STRIKE_IRONCLAD', 3, 17, 0, [(5, 'BOLAS'), (7, 'STRIKE_IRONCLAD')], 64),
             _mark('autoplay_end', 7, 'STRIKE_IRONCLAD', 3, 17, 0, [(5, 'BOLAS')], 52)],
        ]
        self.requests = []

    def ask(self, request):
        answer = super().ask(request)
        if request['cmd'] == 'apply':
            self.requests.append(request)
            answer['presentation_marks'] = self.marks[self.at - 1]
        return answer


@pytest.mark.parametrize('rest_visible', [False, True])
def test_replay_line_splits_autoplays_onto_the_turn_their_draw_opened(monkeypatch, rest_visible):
    fake = _AutoplayReplay(rest_visible)
    monkeypatch.setattr(review, 'RustReplay', fake)
    monkeypatch.setattr(review.canonical_document, 'differential_digest', lambda state: 'final')
    best = {'actions': [{'kind': 'end'}, {'kind': 'play', 'uid': 6, 'target': 0}],
            'final_digest': 'final', 'won': False, 'combat_hp': 17}
    groups = review.replay_line('sim', fake.states[0], best)
    assert all(r['presentation_marks'] is True for r in fake.requests)

    # `actions` holds exactly the decisions, so every (turn, k) address stands.
    assert [(g['turn'], [a['kind'] for a in g['actions']]) for g in groups] == [
        (2, ['end']), (3, ['play'])]
    end = groups[0]['actions'][0]
    # End turn ends where the Strike begins: the enemy turn's damage only.
    assert (end['state_after']['hp'], end['state_after']['enemies'][0]['hp']) == (17, 96)
    assert 'autoplays' not in end

    opening = groups[1]
    assert (opening['hp'], opening['hand'], opening['enemies'][0]['intent']) == (
        17, ['BOLAS', 'POMMEL_STRIKE'], 'PHEROMONE_SPIT_MOVE')
    rows = opening['autoplays']
    assert [(r['source'], r.get('card'), r['state_after']['enemies'][0]['hp']) for r in rows] == (
        [('Hellraiser', 'STRIKE_IRONCLAD', 76), (None, None, 70)] if rest_visible
        else [('Hellraiser', 'STRIKE_IRONCLAD', 76)])
    # The last row ends on the hand the turn opened with.
    assert rows[-1]['state_after']['hand'] == ['BOLAS', 'POMMEL_STRIKE']

    # Mid-turn, the play keeps its own row, cut where Hellraiser's begins.
    play = opening['actions'][0]
    assert play['state_after']['enemies'][0]['hp'] == 64
    assert [(r['card'], r['state_after']['enemies'][0]['hp']) for r in play['autoplays']] == [
        ('STRIKE_IRONCLAD', 52)]


def test_split_autoplays_needs_a_finished_play():
    final = {'hp': 1, 'block': 0, 'energy': 0, 'hand': [], 'enemies': []}
    assert review.split_autoplays([], final) is None
    # A play that parked mid-body finishes in a later apply: no split here.
    begin = _mark('autoplay_begin', 9, 'STRIKE_IRONCLAD', 3, 17, 0, [], 96)
    assert review.split_autoplays([begin], final) is None
