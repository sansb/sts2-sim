"""Recorded MCR inputs and review labels over the Rust transition protocol.

No Python combat transitions. Physical identities are authenticated against
MCR's first shuffled cycle; unsupported/ambiguous choices leave the raw log
available. Only a complete replay matching the recorded outcome is published.
"""
import json
import os
import selectors
import subprocess
import time


class RustReplay:
    def __init__(self, binary, entry, timeout=20):
        self.binary, self.entry, self.timeout = binary, entry, timeout

    def __enter__(self):
        self.buffer = bytearray()
        self.deadline = time.monotonic() + self.timeout
        # Binary unbuffered reads: selector readiness must not miss a line
        # already prefetched into a TextIOWrapper's private buffer.
        self.process = subprocess.Popen([str(self.binary), 'diff-serve'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        try:
            if self.read().get('protocol') != 'diff-serve-v1':
                raise ValueError('unknown Rust replay protocol')
            from rust_exact_solve import canonical_document
            if self.ask({'cmd': 'load', 'entry': self.entry})['digest'] != canonical_document.differential_digest(self.entry):
                raise ValueError('Rust replay root mismatch')
            return self
        except Exception:
            self.__exit__(None, None, None)
            raise

    def read(self):
        while b'\n' not in self.buffer:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise ValueError('Rust replay time budget exceeded')
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise ValueError('Rust replay process ended unexpectedly')
            self.buffer.extend(chunk)
            if len(self.buffer) > 16 * 1024 * 1024:
                raise ValueError('Rust replay response too large')
        line, _, self.buffer = self.buffer.partition(b'\n')
        return json.loads(line)

    def ask(self, request):
        payload = memoryview(json.dumps(request).encode() + b'\n')
        while payload:
            payload = payload[self.process.stdin.write(payload):]
        response = self.read()
        if 'ok' not in response:
            raise ValueError(f'Rust replay refused: {response}')
        return response['ok']

    def __exit__(self, *args):
        self.selector.close()
        self.process.stdin.close()
        self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()


def selection_details(before, wire, after=None, selected_uids=None):
    """Return cards and purpose only where the pending modal identifies them.

    Option ordinals are NOT hand indices for generic selectors. For a single
    upgrade, the exact physical card changed by Rust identifies the answer;
    moving/exhausting suffixes can touch extra cards and are not used to guess.
    """
    pending = before['player'].get('pending') or []
    answer = wire.get('answer', {})
    index = answer.get('index')
    curse_options = _knowledge_demon_curse_options(before)
    if not pending and curse_options is not None:
        if type(index) is int and 0 <= index < len(curse_options):
            return {'cards': [curse_options[index]], 'selection_operation': 'choose',
                    'selection_source': 'KNOWLEDGE_DEMON'}
        return None
    if not pending:
        return None
    kind = pending[0]
    # The Rust consumer resolves physical selections before any downstream
    # draw/exhaust/autoplay hooks. UID order is the actual pick order.
    if selected_uids is not None:
        from rust_review import card_name
        by_uid = {c['uid']: c for pile in before['piles'].values() for c in pile}
        if any(uid not in by_uid for uid in selected_uids):
            raise ValueError('selected physical card missing from replay state')
        source, operation = None, None
        if kind in ('frame_select', 'purity_select'):
            active = by_uid.get(pending[1])
            source = card_name(active) if active else None
            operation = 'exhaust' if kind == 'purity_select' else pending[2][5]
        elif kind == 'potion_select':
            source, operation = pending[1], pending[2][5]
        elif kind == 'stratagem_select':
            source, operation = 'STRATAGEM', ['move', 'hand', 'bottom']
        elif kind == 'foregone_select':
            source, operation = 'FOREGONE_CONCLUSION', ['move', 'hand', 'bottom']
        elif kind == 'turn_start_hand_choice_select':
            source = pending[2].upper()
            operation = 'discard' if source == 'TOOLS_OF_THE_TRADE' else 'exhaust'
        elif kind in ('toasty_mittens_select', 'gambling_chip_select'):
            source = 'TOASTY_MITTENS' if kind == 'toasty_mittens_select' else 'GAMBLING_CHIP'
            operation = 'exhaust' if kind == 'toasty_mittens_select' else 'discard'
        if isinstance(operation, list):
            op = operation[0]
            if op == 'move':
                operation = 'put_on_deck' if operation[1:3] == ['draw', 'top'] else 'return_to_hand'
            else:
                operation = {'exhaust_draw': 'exhaust', 'discard_draw': 'discard',
                             'clone_generated': 'copy', 'move_free_this_turn': 'return_free'}.get(op)
        operation = {'apply_permanent_retain': 'retain', 'free_this_combat': 'make_free'}.get(operation, operation)
        details = {'uids': selected_uids, 'cards': [card_name(by_uid[uid]) for uid in selected_uids]}
        if source and operation:
            details.update(selection_source=source, selection_operation=operation)
        return details
    if type(index) is not int or index < 0:
        return None
    if kind == 'toasty_mittens_select' and index < len(pending[1]):
        uid, payload = pending[1][index]
        return {'uids': [uid], 'cards': [payload[0] + ('+' if payload[1] else '')],
                'selection_operation': 'exhaust', 'selection_source': 'TOASTY_MITTENS'}
    if kind in ('generation_potion_select', 'generation_relic_select'):
        options = pending[2]
        if index < len(options):
            card = options[index]
            return {'cards': [card[0] + ('+' if card[1] else '')],
                    'selection_operation': 'choose', 'selection_source': pending[1]}
        if index == len(options):
            return {'selection_operation': 'skip', 'selection_source': pending[1]}
    if kind in ('abundance_select', 'discovery_select', 'splash_select', 'quasar_select'):
        from rust_review import card_name
        active = next((c for pile in before['piles'].values() for c in pile if c['uid'] == pending[1]), None)
        if active:
            options = pending[3]
            if index < len(options):
                card = options[index]
                return {'cards': [card[0] + ('+' if card[1] else '')],
                        'selection_operation': 'choose', 'selection_source': card_name(active)}
            if index == len(options) and kind != 'abundance_select':
                return {'selection_operation': 'skip', 'selection_source': card_name(active)}
    if kind == 'frame_select' and len(pending) == 3 and after is not None:
        spec = pending[2]
        if spec[:1] != ['select'] or spec[2:4] != [1, 1] or spec[5] != 'upgrade':
            return None
        source = before['piles'].get(spec[1], [])
        following = {c['uid']: c for pile in after['piles'].values() for c in pile}
        changed = [c for c in source if c['uid'] in following and
                   following[c['uid']].get('upgrade', 0) > c.get('upgrade', 0)]
        active = next((c for pile in before['piles'].values() for c in pile if c['uid'] == pending[1]), None)
        if len(changed) == 1 and active:
            from rust_review import card_name
            return {'uids': [changed[0]['uid']], 'cards': [card_name(changed[0])],
                    'selection_operation': 'upgrade', 'selection_source': card_name(active)}
    return None


def _play_is_offered(before, legal, wire):
    """Whether a recorded play names an action Rust's `legal` offers.

    `legal` offers ONE representative uid per group of payload-identical Hand
    cards, while `apply` accepts the physical copy the player actually clicked
    (engine/mod.rs `hand_uids_are_interchangeable`, #2473). The replay must
    apply that exact copy: its identity decides which card moves where, and
    the native pile checkpoints compare order. So a recorded uid absent from
    the list is accepted only when an offered play differs from it solely by
    a Hand uid whose projected card is identical apart from `uid` (the
    projection is the lossless canonical document, so equal payloads are
    equal interned atoms). Under exact piles Rust additionally requires the
    rest of the hand to agree; a copy failing that is its own group and is
    offered verbatim, so it never reaches this fallback (#2805).
    """
    if wire in legal:
        return True
    hand = {c['uid']: {k: v for k, v in c.items() if k != 'uid'}
            for c in before['piles'].get('hand', [])}

    def same(offered, recorded):
        return offered == recorded or (
            offered in hand and recorded in hand and hand[offered] == hand[recorded])

    return any(
        offered.get('kind') == 'play' and set(offered) == set(wire)
        and offered.get('target') == wire.get('target')
        and same(offered['uid'], wire['uid'])
        and ('selection' not in wire or same(offered['selection'], wire['selection']))
        for offered in legal)


# Pending modals opened by `CardSelectCmd::FromChooseACardScreen` (RVA
# 0x1319b0), keyed to the pending slot holding the offered cards. The screen
# records the pick as `PlayerChoiceResult::FromIndex(cards.IndexOf(result))`
# (`<FromChooseACardScreen>d__16::MoveNext` RVA 0x3e58c0, IL_02f6-IL_0304):
# an index into the very list the body passed in, and IndexOf(null) = -1 on
# a skip. `FromIndex` (RVA 0x10d22c) always wraps a present value in a
# one-element list, so a skip is recorded as [-1], never as []. Remote peers
# read it back with `AsIndex` and map a negative to null (IL_03a8-IL_03c4).
# Callers and their canSkip argument: Discovery `<OnPlay>d__4` RVA 0x399254
# IL_0096 ldc.i4.1, Quasar `<OnPlay>d__3` RVA 0x3b4e48 IL_008f ldc.i4.1,
# Splash `<OnPlay>d__2` RVA 0x3be150 IL_0119 ldc.i4.1, Abundance
# `<OnPlay>d__6` RVA 0x3888ec IL_00e7 ldc.i4.0; generation potions pass
# canSkip=true (ENCOUNTER_MECHANICS.md, generation potions). Rust publishes
# the offered cards in that list's order and the skip answer as ordinal
# len(options) (selection_details above). The chosen card is then checked by
# the next native pile checkpoint, so a misordered option list refuses.
_CHOOSE_A_CARD_OPTIONS = {'discovery_select': 3, 'splash_select': 3, 'quasar_select': 3,
                          'abundance_select': 3, 'generation_potion_select': 2}
_CHOOSE_A_CARD_SKIPPABLE = {'discovery_select', 'splash_select', 'quasar_select',
                            'generation_potion_select'}


# Knowledge Demon's CURSE_OF_KNOWLEDGE parks the enemy turn on a blocking
# choice (#3156). `<ChooseCurse>d__39::MoveNext` (RVA 0x3605e0) builds its
# cards as `_curseOfKnowledgeSets[CurseOfKnowledgeCounter]` mapped through
# `CreateCard` in set order (IL_0069-IL_008f) and hands that list to
# `CardSelectCmd::FromChooseACardScreen` with canSkip=false (IL_00a1
# ldc.i4.0, IL_00a2), so the recorded `Index` is `cards.IndexOf(result)` into
# the set (see the FromChooseACardScreen note below) and never -1. The sets
# (`KnowledgeDemon::.cctor` RVA 0xb7e00, IL_0040-IL_00a5) are, per counter,
# [Disintegration, MindRot], [Disintegration, Sloth], [Disintegration,
# WasteAway]. Rust projects the parked choice as the player's
# `enemy_pending` = ['KD_CURSE', options, stage, remaining_uids], its options
# in that same set order (boundary.rs `kd_curse_options_value`), and answers
# ordinal k with option k (engine/turn.rs `resume_kd_curse`). The player's
# own `pending` is empty there: this is a monster's modal, not a card's.
def _knowledge_demon_curse_options(before):
    enemy = before['player'].get('enemy_pending')
    if not isinstance(enemy, list) or not enemy or enemy[0] != 'KD_CURSE':
        return None
    if len(enemy) < 2 or not isinstance(enemy[1], list):
        raise ValueError('Knowledge Demon curse choice has no option list')
    return enemy[1]


def _selection(session, before, result, uid_map):
    legal = session.ask({'cmd': 'legal'})['actions']
    pending = before['player'].get('pending') or []
    curse_options = None if pending else _knowledge_demon_curse_options(before)
    if curse_options is not None:
        if result.get('type') != 'Index':
            raise ValueError('recorded Knowledge Demon curse choice is not an Index')
        indexes = result.get('indexes')
        if not isinstance(indexes, list) or len(indexes) != 1 or type(indexes[0]) is not int:
            raise ValueError('recorded selection needs one display index')
        index = indexes[0]
        # canSkip=false: a -1 (skip) or any index past the set cannot be the
        # native answer.
        if not 0 <= index < len(curse_options):
            raise ValueError('recorded Knowledge Demon curse index outside the offered set')
        wire = {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}
        if wire in legal:
            return wire
        raise ValueError('recorded Knowledge Demon curse choice is not offered by Rust')
    if result.get('type') == 'Index' and pending and pending[0] in (
            *_CHOOSE_A_CARD_OPTIONS, 'generation_relic_select'):
        indexes = result.get('indexes')
        if not isinstance(indexes, list) or len(indexes) != 1 or type(indexes[0]) is not int:
            raise ValueError('recorded selection needs one display index')
        index = indexes[0]
        options_slot = _CHOOSE_A_CARD_OPTIONS.get(pending[0])
        if options_slot is not None:
            options = pending[options_slot]
            if index == -1 and pending[0] in _CHOOSE_A_CARD_SKIPPABLE:
                index = len(options)
            elif not 0 <= index < len(options):
                raise ValueError('recorded selection index outside the offered cards')
        wire = {'kind': 'select', 'answer': {'kind': 'option_index', 'index': index}}
        if wire in legal:
            return wire
    elif result.get('type') == 'CombatCard':
        indexes = result.get('combat_card_indexes')
        if not isinstance(indexes, list) or len(indexes) != 1 or type(indexes[0]) is not int:
            raise ValueError('recorded selection needs one physical card')
        uids = [uid_map(indexes[0])]
        # Rust runs this very rule in-process (`resolve_selection`,
        # `engine::recorded_selection_answer`, #3125): each offered answer
        # decoded through the transition's own enumerator, kept only if it
        # names exactly `uids` and applies, refused when two do. An ordered
        # surface such as Gambling Chip's (326 answers for a 5-card hand)
        # needs no per-answer round trip. A null answer (no Rust decode names
        # the card) falls through to the pile-position rules below.
        resolved = session.ask({'cmd': 'resolve_selection', 'uids': uids})
        if resolved.get('action') is not None:
            return resolved['action']
        candidates = [w for w in legal if w['kind'] == 'select']
        if len(candidates) > 128:
            raise ValueError('recorded selection exceeds replay option budget')
        matches = []
        for wire in candidates:
            details = selection_details(before, wire)
            if details is None:
                # Generic ordinals (frame selects: exhaust, move, discard,
                # upgrade...) are not pile positions. Ask Rust to decode the
                # ordinal through the transition's own option enumerator
                # (`engine::selected_card_uids`, diff_serve
                # `describe_selection`) rather than inferring it from which
                # cards moved (#2961).
                session.ask({'cmd': 'load', 'entry': before})
                try:
                    applied = session.ask({'cmd': 'apply', 'action': wire, 'describe_selection': True})
                except ValueError as exc:
                    # A candidate Rust refuses cannot be the replayed answer;
                    # transport failures (budget, dead process) still raise.
                    if not str(exc).startswith('Rust replay refused'):
                        raise
                    continue
                selected = applied.get('selected_uids')
                details = {'uids': selected} if selected is not None else None
            if details and details.get('uids') == uids:
                matches.append(wire)
        session.ask({'cmd': 'load', 'entry': before})
        if len(matches) == 1:
            return matches[0]
    raise ValueError('recorded selection has no unique supported Rust answer')


def recorded_outcome_matches(actual, won, hp):
    """Whether a replayed terminal agrees with the recorded result.

    A capped post-combat heal leaves `.run` with a band of possible
    combat-end HPs (`final_hp_range`, #1166) instead of one value; the
    replayed HP must then land inside it.
    """
    if won != actual['won']:
        return False
    band = actual.get('final_hp_range')
    if band is None:
        return hp == actual['final_hp']
    return band[0] <= hp <= band[1]


def recorded_witness(binary, entry, replay, actual):
    """Resolve every recorded decision to a legal Rust action, without search.

    UID equality is accepted only after checking the entire entry deck against
    the recorded raw shuffle. Generated cards then share the creation allocator.
    The source capture is already fight-associated by the worker.
    """
    import mcr_native
    from rust_exact_solve import canonical_document
    if replay.get('version') != 'v0.111.0':
        raise ValueError('recorded replay build mismatch')
    events = replay.get('events')
    if not isinstance(events, list) or not events or len(events) > 2000:
        raise ValueError('recorded input count outside replay budget')
    instances = mcr_native.first_cycle_from_replay(replay)
    cards = {c['uid']: c for pile in entry['piles'].values() for c in pile}
    # Ordinary roots number the dealt deck by instance index, so recorded
    # combat_card_index k is uid k. A Thieving Hopper root instead keeps each
    # deck card's master-deck row as its uid (the frozen `start_combat`'s
    # `PhysicalCard(card, deck_row)`: DeckVersion is an object relation), and
    # its projection says so by carrying `hopper_master_deck`. There instance
    # k is row first_cycle_rows_from_replay(replay)[k] (#2805). Cards created
    # in combat share the allocator after the deck in both numberings.
    rows = None
    if 'hopper_master_deck' in entry['player']:
        rows = mcr_native.first_cycle_rows_from_replay(replay)
        master = entry['player']['hopper_master_deck']
        if (sorted(row for row, _ in master) != list(range(len(rows)))
                or entry['player'].get('next_card_uid') != len(rows)):
            raise ValueError('recorded master-deck rows differ from the Rust root')

    def physical_uid(index):
        return rows[index] if rows is not None and index < len(rows) else index
    # Whispering Earring's turn-one loop (#3414) AutoPlays dealt cards before
    # the root, and a played Power leaves every pile. Only there may a deck
    # uid be absent; every present card is still matched, and a recorded
    # play of an absent uid fails the per-input identity check below.
    earring = 'RELIC.WHISPERING_EARRING' in (entry['player'].get('relics_entering') or ())
    if any((physical_uid(i) not in cards and not earring)
           or (physical_uid(i) in cards
               and (cards[physical_uid(i)]['id'], cards[physical_uid(i)].get('upgrade', 0)) != tuple(card[:2]))
           for i, card in enumerate(instances)):
        raise ValueError('recorded initial physical card identities differ')

    def uid_map(index):
        if type(index) is not int or index < 0:
            raise ValueError('invalid recorded physical card index')
        return physical_uid(index)
    native = NativeChecks(replay)
    wires, consumed = [], set()
    before = entry
    with RustReplay(binary, entry) as session:
        for index, event in enumerate(events):
            if index in consumed:
                continue
            native.start(event)
            action = event.get('action', {})
            kind = action.get('type')
            wire = None
            if kind in ('NetPlayCardAction', 'NetUsePotionAction'):
                if kind == 'NetPlayCardAction':
                    uid = uid_map(action['combat_card_index'])
                    card = next((c for c in before['piles']['hand'] if c['uid'] == uid), None)
                    if card is None or card['id'] != action['card_id'].removeprefix('CARD.'):
                        raise ValueError(f'recorded card identity mismatch at input {index}')
                    wire = {'kind': 'play', 'uid': uid}
                else:
                    wire = {'kind': 'potion', 'slot': action['potion_index']}
                tid = action.get('target_id')
                if tid and action.get('target_player_id') is None:
                    targets = [i for i, m in enumerate(before['monsters']) if m.get('uid', 0) == tid - 1]
                    if len(targets) != 1:
                        raise ValueError('recorded target identity mismatch')
                    wire['target'] = targets[0]
                legal = session.ask({'cmd': 'legal'})['actions']
                if kind == 'NetPlayCardAction':
                    offered = _play_is_offered(before, legal, wire)
                else:
                    offered = wire in legal
                if not offered and kind == 'NetPlayCardAction':
                    # Immediate exhaust choices (e.g. Burning Pact) belong to
                    # the play wire. Consume only its adjacent recorded choice.
                    following = events[index + 1] if index + 1 < len(events) else {}
                    selected = following.get('result', {}).get('combat_card_indexes', [])
                    if following.get('event_type') == 'PlayerChoice' and len(selected) == 1:
                        combined = dict(wire, selection=uid_map(selected[0]))
                        if _play_is_offered(before, legal, combined):
                            wire = combined
                            consumed.add(index + 1)
                            offered = True
                if not offered:
                    raise ValueError(f'recorded action is not legal at input {index}')
            elif kind == 'NetEndPlayerTurnAction':
                if action.get('turn_number') != before['player'].get('turn', 1):
                    raise ValueError('recorded turn number mismatch')
                wire = {'kind': 'end'}
            elif event.get('event_type') == 'PlayerChoice':
                wire = _selection(session, before, event.get('result', {}), uid_map)
            elif event.get('event_type') in ('HookAction', 'ResumeAction') or kind == 'NetReadyToBeginEnemyTurnAction':
                native.no_decision(event)
                continue
            else:
                raise ValueError(f'unsupported recorded event at input {index}')
            if before['player'].get('over'):
                raise ValueError('recorded inputs continue after simulated combat end')
            applied = session.ask({'cmd': 'apply', 'action': wire, 'native_checkpoints': True})
            played = native.stage(applied)
            before = session.ask({'cmd': 'project'})['state']
            wires.append(wire)
            if wire['kind'] != 'end':
                native.completed(played if played is not None else before)
    native.finish()
    # `won` is the combat result, as `.run` records it (`actual_outcome`:
    # the player survived). A Battleworn Dummy timeout is one: the timer's
    # escape empties the roster and CheckWinCondition takes the victory path,
    # while the `.run` shows it only by the event's missing reward (#3369).
    # So both sides say won here; the kill-over-timeout preference lives in
    # the scorers, not in this recorded-outcome check.
    won = before['player'].get('hp', 0) > 0 and all(m.get('hp', 0) <= 0 for m in before['monsters'])
    if not before['player'].get('over'):
        raise ValueError('recorded inputs end before combat completes')
    if not recorded_outcome_matches(actual, won, before['player'].get('hp', 0)):
        raise ValueError('Rust recorded replay outcome differs from recorded result')
    return {'native_checkpoints': native.validated, 'actions': wires, 'won': won, 'combat_hp': before['player'].get('hp', 0),
            'final_digest': canonical_document.differential_digest(before)}


# Native MONSTER classes the Rust engine carries as one aggregate MonsterKind,
# mirroring `AGGREGATE_MONSTER_CLASSES` in engine/tools/generate_content.py.
# Codegen refuses an aggregate unless every class behind it agrees on every
# modeled fact, so the class name carries no state that the checkpoint's
# positional HP/max-HP/block comparison could miss (#2805). Drift in either
# direction fails closed as a monster mismatch: a new aggregate absent here,
# or this aggregate split into per-class Rust kinds.
_AGGREGATE_MONSTER_CLASSES = {
    'DECIMILLIPEDE_SEGMENT': ('DECIMILLIPEDE_SEGMENT_FRONT', 'DECIMILLIPEDE_SEGMENT_MIDDLE',
                              'DECIMILLIPEDE_SEGMENT_BACK'),
}
_NATIVE_MONSTER_KIND = {native: kind for kind, natives in _AGGREGATE_MONSTER_CLASSES.items()
                        for native in natives}


#: Native player power ids whose canonical player field is NOT the id with
#: `_POWER` stripped and lowercased (#3029). The player-side debuffs are
#: their own canonical slots (`ids.rs` `PowerId`: `player_weak`,
#: `player_vuln`, `player_frail`, `player_shrink`; `boundary.rs`
#: `PlayerSlot::PlayerDoom` / `PlayerRitual`); the names that bare would
#: collide with a monster slot are spelled `*_power` (`hex_power`,
#: `haunt_power`). Shrink's is a presence sentinel, so it is a projection
#: (`_shrink`) instead.
_NATIVE_PLAYER_POWER_RENAMES = {
    'WEAK_POWER': 'player_weak',
    'VULNERABLE_POWER': 'player_vuln',
    'FRAIL_POWER': 'player_frail',
    'DOOM_POWER': 'player_doom',
    'RITUAL_POWER': 'player_ritual',
    'HEX_POWER': 'hex_power',
    'HAUNT_POWER': 'haunt_power',
    'THE_SEALED_THRONE_POWER': 'sealed_throne',
    'DRAW_CARDS_NEXT_TURN_POWER': 'draw_next_turn',
    # `steps/templates.rs` `free_ethereal`: "the next n Ethereal card
    # (VeilpiercerPower) plays cost zero".
    'VEILPIERCER_POWER': 'free_ethereal',
    # #3159: the bare `self_forming_clay` field is the relic-ownership flag,
    # so Rust's `PowerId::SelfFormingClayPower` takes the `*_power` spelling.
    'SELF_FORMING_CLAY_POWER': 'self_forming_clay_power',
}

#: Native player power ids Rust carries under the stripped-lowercase name:
#: every native `PowerModel` subclass (TypeDef `Extends` chain, v0.111.0 DLL
#: `9cb4f1ad`) whose name is a `PowerId` canonical name (`ids.rs`
#: `PowerId::NAMES`) or a scalar `boundary.rs` player spec (`clarity`,
#: `duplication`, `gigantification`, `hammer_time`, `no_energy_gain`,
#: `radiance`, `regen`, `ringing`, `the_hunt`, `toric_toughness`). Powers
#: only monsters carry are included on purpose: absent on both sides they
#: compare equal, and should one ever reach the player it is compared rather
#: than skipped. A Bool slot compares as 0/1. (ImitationLearningPower shares
#: a name with a list-valued spec, so it is left out and reports
#: `power_not_projected`.)
_NATIVE_PLAYER_POWER_DIRECT = frozenset({
    'ACCELERANT_POWER', 'ACCURACY_POWER', 'ADAPTABLE_POWER',
    'AFTERIMAGE_POWER', 'AGGRESSION_POWER', 'ARSENAL_POWER', 'ARTIFACT_POWER',
    'ASLEEP_POWER', 'BARRICADE_POWER', 'BEACON_OF_HOPE_POWER',
    'BIASED_COGNITION_POWER', 'BLACK_HOLE_POWER', 'BLOCK_NEXT_TURN_POWER',
    'BLUR_POWER', 'BORROWED_TIME_POWER', 'BUFFER_POWER', 'BURROWED_POWER',
    'BURST_POWER', 'CACOPHONY_POWER', 'CALAMITY_POWER', 'CALCIFY_POWER',
    'CALL_OF_THE_VOID_POWER', 'CHAINS_OF_BINDING_POWER',
    'CHILD_OF_THE_STARS_POWER', 'CLARITY_POWER', 'COLOSSUS_POWER',
    'CONQUEROR_POWER', 'CONSUMING_SHADOW_POWER', 'COOLANT_POWER',
    'CORROSIVE_WAVE_POWER', 'CORRUPTION_POWER', 'COUNTDOWN_POWER',
    'CREATIVE_AI_POWER', 'CRIMSON_MANTLE_POWER', 'CRUELTY_POWER',
    'CURIOUS_POWER', 'CURL_UP_POWER', 'DANSE_MACABRE_POWER', 'DARK_EMBRACE_POWER',
    'DEBILITATE_POWER', 'DEMESNE_POWER', 'DEMISE_POWER', 'DEMON_FORM_POWER',
    'DEVOUR_LIFE_POWER', 'DISINTEGRATION_POWER', 'DOUBLE_DAMAGE_POWER',
    'DUPLICATION_POWER', 'ECHO_FORM_POWER', 'ENERGY_NEXT_TURN_POWER',
    'ENRAGE_POWER', 'ENTROPY_POWER', 'ENVENOM_POWER', 'ESCAPE_ARTIST_POWER',
    'FAN_OF_KNIVES_POWER', 'FASTEN_POWER', 'FEEL_NO_PAIN_POWER',
    'FERAL_POWER', 'FLAME_BARRIER_POWER', 'FLUTTER_POWER',
    'FREE_ATTACK_POWER', 'FREE_POWER_POWER', 'FREE_SKILL_POWER',
    'FRIENDSHIP_POWER', 'FURNACE_POWER', 'GENESIS_POWER',
    'GIGANTIFICATION_POWER', 'HAILSTORM_POWER', 'HAMMER_TIME_POWER',
    'HANG_POWER', 'HARD_TO_KILL_POWER', 'HELLO_WORLD_POWER',
    'HELLRAISER_POWER', 'HIBERNATE_POWER', 'HIGH_VOLTAGE_POWER',
    'INFERNO_POWER', 'INFINITE_BLADES_POWER', 'INTANGIBLE_POWER',
    'ITERATION_POWER', 'JUGGERNAUT_POWER', 'JUGGLING_POWER',
    'KNOCKDOWN_POWER', 'LETHALITY_POWER', 'LIGHTNING_ROD_POWER', 'LOOP_POWER',
    'MACHINE_LEARNING_POWER', 'MASTER_PLANNER_POWER', 'MAYHEM_POWER',
    'MIND_ROT_POWER', 'MONARCHS_GAZE_POWER', 'NECRO_MASTERY_POWER',
    'NEMESIS_POWER', 'NEUROSURGE_POWER', 'NOSTALGIA_POWER',
    'NOXIOUS_FUMES_POWER', 'NO_BLOCK_POWER', 'NO_DRAW_POWER',
    'NO_ENERGY_GAIN_POWER', 'OBLIVION_POWER', 'ONE_FOR_ALL_POWER',
    'ONE_TWO_PUNCH_POWER', 'ORBIT_POWER', 'PAGESTORM_POWER',
    'PAINFUL_STABS_POWER', 'PALE_BLUE_DOT_POWER', 'PARRY_POWER',
    'PHANTOM_BLADES_POWER', 'PILLAR_OF_CREATION_POWER', 'PLATING_POWER',
    'POISON_POWER', 'PREP_TIME_POWER', 'PYRE_POWER', 'RADIANCE_POWER',
    'RAGE_POWER', 'RAMPART_POWER', 'RAVENOUS_POWER', 'REAPER_FORM_POWER',
    'REBOUND_POWER', 'REFLECT_POWER', 'REGEN_POWER', 'RETAIN_HAND_POWER',
    'RINGING_POWER', 'ROLLING_BOULDER_POWER', 'RUPTURE_POWER',
    'SANDPIT_POWER', 'SEEKING_EDGE_POWER', 'SENTRY_MODE_POWER',
    'SERPENT_FORM_POWER', 'SHADOWMELD_POWER', 'SHADOW_STEP_POWER',
    'SHRIEK_POWER', 'SHROUD_POWER', 'SIC_EM_POWER', 'SIGNAL_BOOST_POWER',
    'SKITTISH_POWER', 'SLEIGHT_OF_FLESH_POWER', 'SLIPPERY_POWER',
    'SLOTH_POWER', 'SLOW_POWER', 'SLUMBER_POWER', 'SMOGGY_POWER',
    'SMOKESTACK_POWER', 'SNEAKY_POWER', 'SOAR_POWER', 'SPECTRUM_SHIFT_POWER',
    'SPEEDSTER_POWER', 'SPINNER_POWER', 'SPIRIT_OF_ASH_POWER',
    'STAMPEDE_POWER', 'STAR_NEXT_TURN_POWER', 'STOCK_POWER', 'STORM_POWER',
    'STRANGLE_POWER', 'STRATAGEM_POWER', 'SUBROUTINE_POWER', 'SUCK_POWER',
    'SUMMON_NEXT_TURN_POWER', 'SWORD_SAGE_POWER', 'TAINTED_POWER',
    'TANGLED_POWER', 'TENDER_POWER', 'TERRITORIAL_POWER', 'THE_GAMBIT_POWER',
    'THE_HUNT_POWER', 'THORNS_POWER', 'THUNDER_POWER',
    'TOOLS_OF_THE_TRADE_POWER', 'TORIC_TOUGHNESS_POWER', 'TRACKING_POWER',
    'TRASH_TO_TREASURE_POWER', 'TYRANNY_POWER', 'UNDERWORLD_POWER',
    'UNMOVABLE_POWER', 'VICIOUS_POWER', 'VIGOR_POWER', 'VOID_FORM_POWER',
    'WASTE_AWAY_POWER', 'WELL_LAID_PLANS_POWER', 'WRAITH_FORM_POWER',
})

#: `native id -> canonical player field` for every power compared as that
#: one field's value.
NATIVE_PLAYER_POWER_FIELDS = dict(
    {name: name[:-len('_POWER')].lower() for name in _NATIVE_PLAYER_POWER_DIRECT},
    **_NATIVE_PLAYER_POWER_RENAMES)


def _rust_scalar(player, field, default=0):
    value = player.get(field, default)
    return int(value) if isinstance(value, bool) else value


def _with_temporary(base, temporary):
    """The native stat row: Rust's base plus its temporary part.

    A `Temporary*Power` applies its stat through the ordinary stat power
    (v0.111.0 `get_InternallyAppliedPower`: TemporaryStrengthPower RVA
    `0xa9ad3`, TemporaryDexterityPower `0xa95dc`, TemporaryFocusPower
    `0xa9853`, applied from `BeforeApplied` `0xa9c0c` / `0xa9714` /
    `0xa9994`), so the native STRENGTH/DEXTERITY/FOCUS row already includes
    it. Rust keeps the permanent part in `strength` and the part to undo in
    `temp_strength` (likewise dexterity, focus).
    """
    return lambda player: _rust_scalar(player, base) + _rust_scalar(player, temporary)


#: Native Strength: base + temporary part. Red Skull's 3 is already in
#: Rust's `strength` (#3044): the relic applies and removes a real
#: StrengthPower as the owner crosses its threshold
#: (`<ModifyStrengthIfNecessary>d__14::MoveNext` RVA `0x32f790` IL_00b8 and
#: IL_013b), and `engine/damage.rs` `red_skull_after_player_hp_changed` now
#: writes it at those transitions instead of adding it at damage time.
_strength = _with_temporary('strength', 'temp_strength')


def _shrink(player):
    """Rust's presence sentinel back in native terms.

    The Shrinker Beetle applies ShrinkPower with amount -1
    (`<ShrinkMove>d__12::MoveNext` RVA `0x369450` IL_00ba
    `ldsfld Decimal::MinusOne`); Rust carries that one infinite instance as
    `player_shrink = 1` (`moves/shared.rs` `shrink`).
    """
    value = _rust_scalar(player, 'player_shrink')
    return -1 if value == 1 else value


#: `native id -> (player -> native-equivalent amount)` for the powers Rust
#: carries in some other shape than one same-valued field.
NATIVE_PLAYER_POWER_PROJECTIONS = {
    'STRENGTH_POWER': _strength,
    'DEXTERITY_POWER': _with_temporary('dexterity', 'temp_dexterity'),
    'FOCUS_POWER': _with_temporary('focus', 'temp_focus'),
    'SHRINK_POWER': _shrink,
    # Slithering Strangler's Constrict: `[owner uid, amount]` per source
    # (`boundary.rs` `PlayerSlot::ConstrictSources`).
    'CONSTRICT_POWER': lambda player: sum(
        row[1] for row in player.get('constrict_sources', [])),
    # Magi Knight's one-caster Dampen (`hot.rs` Dampen snapshot): one
    # application per caster row.
    'DAMPEN_POWER': lambda player: len(player.get('dampen_casters', [])),
    # `SurroundedPower.Facing`, or -1 when no SurroundedPower exists
    # (`hot.rs` `kaiser_facing`, native `get_Facing` `0xa8bac`); the power's
    # amount is 1.
    'SURROUNDED_POWER': lambda player: int(
        _rust_scalar(player, 'kaiser_facing', -1) != -1),
}

#: The three `Temporary*Power` families, each one canonical signed sum
#: (`temp_strength`, `temp_dexterity`, `temp_focus`). v0.111.0 (DLL
#: `9cb4f1ad`): each family's `get_Sign` (TemporaryStrengthPower RVA
#: `0xa9add`, TemporaryDexterityPower `0xa95e6`, TemporaryFocusPower
#: `0xa985d`) returns `IsPositive ? 1 : -1`; the base `get_IsPositive`
#: (`0xa9ada` / `0xa95e3` / `0xa985a`) is `ldc.i4.1`, and each member listed
#: with sign -1 overrides it with `ldc.i4.0` IL_0001 (e.g.
#: HyperbeamFocusDownPower `0xa3a41`, ShacklingPotionPower `0xa74d9`).
#: Membership is the TypeDef `Extends` row of every subclass.
NATIVE_TEMPORARY_POWER_FAMILIES = {
    'temp_strength': {
        'COORDINATE_POWER': 1, 'FEEDING_FRENZY_POWER': 1,
        'FLEX_POTION_POWER': 1, 'REPTILE_TRINKET_POWER': 1,
        'SETUP_STRIKE_POWER': 1,
        'CRUSH_UNDER_POWER': -1, 'DARK_SHACKLES_POWER': -1,
        'DYING_STAR_POWER': -1, 'ENFEEBLING_TOUCH_POWER': -1,
        'MANGLE_POWER': -1, 'MOCK_TEMPORARY_STRENGTH_LOSS_POWER': -1,
        'MONARCHS_GAZE_STRENGTH_DOWN_POWER': -1, 'PIERCING_WAIL_POWER': -1,
        'SHACKLING_POTION_POWER': -1,
    },
    'temp_dexterity': {
        'ANTICIPATE_POWER': 1, 'FADE_POWER': 1, 'HELICAL_DART_POWER': 1,
        'SPEED_POTION_POWER': 1,
    },
    'temp_focus': {
        'FOCUSED_STRIKE_POWER': 1, 'HOTFIX_POWER': 1, 'SYNCHRONIZE_POWER': 1,
        'HYPERBEAM_FOCUS_DOWN_POWER': -1,
    },
}
_NATIVE_TEMPORARY_POWER = {name: (field, sign)
                           for field, members in NATIVE_TEMPORARY_POWER_FAMILIES.items()
                           for name, sign in members.items()}


def _automation_instances(player):
    """Per-object amounts: the first live object, then the later rows (#3021)."""
    later = [row[0] for row in player.get('automation_later_instances', [])]
    total = player.get('automation', 0)
    if not total:
        return [] if not later else [0] + later
    return [total - sum(later)] + later


#: Native `PowerInstanceType.Instanced` player powers Rust carries one row
#: per native object, with that object's amount (v0.111.0 `get_InstanceType`
#: IL_0001 `ldc.i4.1`: AutomationPower RVA `0x9fa0e`, MonologuePower
#: `0xa4b15`, PanachePower `0xa5700`, TheBombPower `0xa9edd`). Compared as
#: the ordered amount list, so the instance count is compared too.
NATIVE_INSTANCED_PLAYER_POWERS = {
    'AUTOMATION_POWER': _automation_instances,
    'MONOLOGUE_POWER': lambda player: [row[1] for row in player.get('monologue_instances', [])],
    'PANACHE_POWER': lambda player: [row[1] for row in player.get('panache_instances', [])],
    'THE_BOMB_POWER': lambda player: [row[1] for row in player.get('the_bomb_instances', [])],
}

#: Powers with a non-default `get_InstanceType` (IL_0001 `ldc.i4.1`:
#: CacophonyPower `0xa014b`, KnockdownPower `0xa44d3`, OrbitPower
#: `0xa5476`, RollingBoulderPower `0xa6da1`, SandpitPower `0xa6fc9`,
#: ToricToughnessPower `0xaa24b`; `ldc.i4.2`: OblivionPower `0xa51b5`,
#: StranglePower `0xa87c5`) that Rust carries as ONE scalar. One native
#: object compares exactly; two or more cannot be told apart from their
#: sum, so that checkpoint reports `power_instances_not_projected` rather
#: than comparing the sum.
NATIVE_SCALAR_INSTANCED_PLAYER_POWERS = frozenset({
    'CACOPHONY_POWER', 'KNOCKDOWN_POWER', 'OBLIVION_POWER', 'ORBIT_POWER',
    'ROLLING_BOULDER_POWER', 'SANDPIT_POWER', 'STRANGLE_POWER',
    'TORIC_TOUGHNESS_POWER',
})


class PowerMismatch(ValueError):
    """A native player power the Rust state disagrees with or cannot express.

    `kind` is `amount` (a scalar or signed-family sum differs), `instances`
    (an Instanced power's ordered per-object amounts differ, count
    included), `power_not_projected` (a native id this comparison has no
    canonical field for) or `power_instances_not_projected` (several
    objects of an Instanced power Rust carries as one scalar). `power` is
    the native id (or the family's canonical field), `native`/`rust` the
    compared values.
    """

    def __init__(self, kind, power, native, rust):
        self.kind, self.power, self.native, self.rust = kind, power, native, rust
        if kind in ('power_not_projected', 'power_instances_not_projected'):
            text = f'recorded replay cannot compare native player power {power} ({kind})'
        else:
            text = (f'recorded replay differs from native player powers: '
                    f'{power} native {native} rust {rust}')
        super().__init__(text)

    def as_row(self):
        return {'class': self.kind, 'power': self.power,
                'native': self.native, 'rust': self.rust}


def native_player_power_mismatches(state, native):
    """Every disagreement between the hero's native powers and Rust's player.

    Returns `PowerMismatch` objects, native order first, then fields Rust
    holds that the native list lacks. Never raises for a disagreement; a
    malformed checkpoint raises `ValueError`.
    """
    heroes = [c for c in native['creatures'] if c.get('player_id') is not None]
    if len(heroes) != 1 or not isinstance(heroes[0].get('powers'), list):
        raise ValueError('native checkpoint requires one player power list')
    player = state['player']
    scalars, families, instanced, out = {}, {}, {}, []
    for power in heroes[0]['powers']:
        name, amount = power.get('id'), power.get('amount')
        if not isinstance(amount, int) or isinstance(amount, bool):
            raise ValueError('malformed native player power list')
        if name in NATIVE_INSTANCED_PLAYER_POWERS:
            instanced.setdefault(name, []).append(amount)
        elif name in _NATIVE_TEMPORARY_POWER:
            field, sign = _NATIVE_TEMPORARY_POWER[name]
            families[field] = families.get(field, 0) + sign * amount
        elif name in NATIVE_PLAYER_POWER_FIELDS or name in NATIVE_PLAYER_POWER_PROJECTIONS:
            if name in scalars:
                if name not in NATIVE_SCALAR_INSTANCED_PLAYER_POWERS:
                    raise ValueError(f'native player power {name} repeats')
                out.append(PowerMismatch('power_instances_not_projected', name, None, None))
                scalars[name] = None
                continue
            scalars[name] = amount
        else:
            out.append(PowerMismatch('power_not_projected', name, amount, None))
    projections = {name: (lambda player, field=field: _rust_scalar(player, field))
                   for name, field in NATIVE_PLAYER_POWER_FIELDS.items()}
    projections.update(NATIVE_PLAYER_POWER_PROJECTIONS)
    for name, project in sorted(projections.items()):
        if scalars.get(name, 0) is None:
            continue
        expected, actual = scalars.get(name, 0), project(player)
        if expected != actual:
            out.append(PowerMismatch('amount', name, expected, actual))
    for field in sorted(NATIVE_TEMPORARY_POWER_FAMILIES):
        expected, actual = families.get(field, 0), _rust_scalar(player, field)
        if expected != actual:
            out.append(PowerMismatch('amount', field, expected, actual))
    for name, project in sorted(NATIVE_INSTANCED_PLAYER_POWERS.items()):
        expected, actual = instanced.get(name, []), project(player)
        if expected != actual:
            out.append(PowerMismatch('instances', name, expected, actual))
    return out


def check_native_player_powers(state, native):
    """Raise the first `PowerMismatch` of a native checkpoint (#3029)."""
    mismatches = native_player_power_mismatches(state, native)
    if mismatches:
        raise mismatches[0]


#: Native `OrbModel` ids (`OrbState.id`) → the canonical `orbs` kind Rust
#: projects (`[kind, value]`, `boundary.rs` `PlayerSlot::Orbs`). An orb id
#: outside this table can never equal a Rust orb, so it reads as an `orbs`
#: mismatch rather than being skipped.
_NATIVE_ORB_KIND = {
    'LIGHTNING_ORB': 'LIGHTNING', 'FROST_ORB': 'FROST', 'DARK_ORB': 'DARK',
    'PLASMA_ORB': 'PLASMA', 'GLASS_ORB': 'GLASS',
}


def _native_player_field_mismatch(state, p):
    """The first `PlayerState` field Rust carries exactly that disagrees (#3335).

    Returns `(field, native, rust)` or None. Every value is what
    `NetFullCombatState::FromRun` (v0.111.0 `sts2.dll`, RVA 0x119988) writes
    for the player:

    * `gold` — `Player::get_Gold`, IL_02c8-02cd; Rust `player.gold`.
    * `stars` — `PlayerCombatState::get_Stars`, IL_02ac-02b1; Rust
      `player.stars`.
    * `potion_slots` — `max_potion_count`, `Player::get_MaxPotionCount`
      (IL_02ba-02bf), which is `_potionSlots.Count` (RVA 0x11662d); Rust's
      authoritative sparse `player.potion_slots`, compared by length.
    * `potions` — `Player::get_Potions` (IL_041a-0450), `_potionSlots`
      filtered by `p != null` (RVA 0x1166f0, predicate `<get_Potions>b__74_0`
      RVA 0x3d915e) in slot order; Rust's non-null `potion_slots` in order.
    * `orbs` — `OrbQueue::get_Orbs` in queue order (IL_03e2-0413), compared
      by orb kind only.

    Deliberately NOT compared (Rust's state has no exact counterpart):
    `phase` (phase timing is outside this check, and Rust's integer
    `player_phase` is a different encoding); orb `passive`/`evoke` (native's
    Focus-modified display values, while Rust carries base orb state);
    `relics` (Rust keeps its admission-time `relics_entering` order and
    per-relic counter slots, not a live relic list, and relic `props` have no
    generic mapping); `character`, `player_id` (identity, fixed at
    admission); `player_rng` and `relic_grab_bag` (out-of-combat streams
    Rust does not carry). Card fields other than id/upgrade (`energy_cost`,
    `affliction`, `keywords`, `enchantment`, `props`, `floor_added_to_deck`)
    are pile contents outside this player-field comparison.
    """
    player = state['player']
    slots = player.get('potion_slots', [])
    orbs = player.get('orbs', [])
    fields = (
        ('gold', p['gold'], player.get('gold', 0)),
        ('stars', p['stars'], player.get('stars', 0)),
        ('potion_slots', p['max_potion_count'], len(slots)),
        ('potions', list(p['potions']), [s for s in slots if s is not None]),
        ('orbs', [_NATIVE_ORB_KIND.get(o['id'], o['id']) for o in p['orbs']],
         [o[0] for o in orbs]),
    )
    for field, native, rust in fields:
        if native != rust:
            return field, native, rust
    return None


def check_native_snapshot(state, native, *, powers=False):
    """Validate synchronous completed-action checkpoints, not phase timing.

    Card cost serialization is outside this visible-state + RNG check. The
    player's powers are compared only when `powers` is true
    (`check_native_player_powers`, #3029): the production review keeps its
    pre-#3029 verdicts, and the census reports power drift beside its
    certification verdict. The rendered snapshot still comes entirely from
    Rust, never spliced from MCR. Gold, stars, the potion belt and the orb
    queue (`_native_player_field_mismatch`, #3335) are compared after the
    RNG, each as its own named field.
    """
    players = native['players']
    heroes = [c for c in native['creatures'] if c.get('player_id') is not None]
    if len(players) != 1 or len(heroes) != 1:
        raise ValueError('native checkpoint requires one player')
    p, h = players[0], heroes[0]
    resources = {'hp': h['current_hp'], 'max_hp': h['max_hp'], 'block': h['block'],
                 'energy': p['energy'], 'turn': p['turn_number']}
    defaults = {'energy': 3, 'turn': 1}
    if any(state['player'].get(k, defaults.get(k, 0)) != v for k, v in resources.items()):
        raise ValueError('recorded replay differs from native player state')
    monsters = [c for c in native['creatures'] if c.get('monster_id') is not None]
    if [(m['kind'], m.get('hp', 0), m.get('max_hp', 0), m.get('block', 0)) for m in state['monsters'] if m.get('hp', 0) > 0] != [
            (_NATIVE_MONSTER_KIND.get(m['monster_id'], m['monster_id']), m['current_hp'], m['max_hp'], m['block'])
            for m in monsters if m['current_hp'] > 0]:
        raise ValueError('recorded replay differs from native monsters')
    for pile in p['piles']:
        if [[c['id'], c.get('upgrade', 0)] for c in state['piles'].get(pile['pile_type'].lower(), [])] != [
                [c['card']['id'], c['card']['upgrade_level']] for c in pile['cards']]:
            raise ValueError('recorded replay differs from native card piles')
    # The simulator-free home of the checkpoint's RNG rows (#2999); the same
    # objects `mcr_validate` re-exports.
    import mcr_native
    for attr, stream in mcr_native._REPRESENTED_RNG_STREAMS.items():
        expected = mcr_native._rng_tuple(native['rng'], stream)
        actual = state['rng'][attr]
        if list(expected[:4]) != actual['words'] or expected[4] != actual['counter']:
            raise ValueError('recorded replay differs from native RNG')
    mismatch = _native_player_field_mismatch(state, p)
    if mismatch is not None:
        field, expected, actual = mismatch
        raise ValueError(f'recorded replay differs from native {field}: '
                         f'native {expected} rust {actual}')
    if powers:
        check_native_player_powers(state, native)


#: `diff-serve` apply `native_checkpoints` kinds (#3242): native
#: completed-action checkpoints that fall INSIDE one Rust wire action.
#: `void_form_end_turn_request` is the Void Form play's own checkpoint (its
#: OnPlay only marks the player ready to end the turn; Rust ends it in the
#: same apply). `auto_post_hook_finished` is the state after the whole
#: AfterAutoPostPlayPhaseEntered dispatch, which native runs as one
#: GenericHookGameAction when a listener's task yields.
VOID_FORM_END_TURN_REQUEST = 'void_form_end_turn_request'
AUTO_POST_HOOK_FINISHED = 'auto_post_hook_finished'


class NativeChecks:
    def __init__(self, replay):
        self.checks = [c for c in replay.get('checksums', [])
                       if c.get('context', '').startswith('finished action execution ')]
        self.cursor = self.validated = 0
        self.owner = None
        self.auto_post = None

    def stage(self, applied):
        """Split one apply's intermediate native checkpoint states (#3242).

        Returns the state the applied wire's OWN completed-action checkpoint
        is compared against when the wire ran past it (Void Form's play,
        before the EndTurn it requested), else None. Keeps the AutoPost hook
        state for the recorded HookAction that names that checkpoint; the
        next apply or the enemy-turn ready marker drops it unused, since a
        dispatch whose listeners never yield writes no checkpoint. A kind
        recorded twice, an unknown kind, or a state that does not project
        refuses: nothing here skips a checkpoint.
        """
        recorded = applied.get('native_checkpoints')
        if not isinstance(recorded, list):
            raise ValueError('apply did not report its native checkpoints')
        states = {}
        for entry in recorded:
            kind = entry.get('kind')
            if kind not in (VOID_FORM_END_TURN_REQUEST, AUTO_POST_HOOK_FINISHED):
                raise ValueError(f'unknown native checkpoint kind {kind!r}')
            if kind in states:
                raise ValueError(f'native checkpoint {kind} recorded twice in one action')
            if entry.get('state') is None:
                raise ValueError(f'native checkpoint {kind} does not project: '
                                 f"{entry.get('refusal')}")
            states[kind] = entry['state']
        self.auto_post = states.get(AUTO_POST_HOOK_FINISHED)
        return states.get(VOID_FORM_END_TURN_REQUEST)

    def no_decision(self, event):
        """A recorded event with no Rust wire (hook, resume, ready marker).

        A HookAction after the apply that passed the AutoPost dispatch, and
        before the enemy-turn ready marker, IS that dispatch's
        GenericHookGameAction (`CombatManager/<EndPlayerTurnPhaseOneInternal>
        d__130` RVA 0x3f4a4c IL_020c-023c, #3242): its checkpoint is compared
        against the staged state now. Any other HookAction keeps its owner
        for the PlayerChoice that follows it.
        """
        if event.get('event_type') == 'HookAction':
            state, self.auto_post = self.auto_post, None
            if state is not None:
                self.completed(state)
        elif (event.get('action') or {}).get('type') == 'NetReadyToBeginEnemyTurnAction':
            self.auto_post = None

    def start(self, event):
        action = event.get('action', {})
        kind = action.get('type')
        if kind == 'NetPlayCardAction':
            self.owner = ('PlayCardAction card:', f" index: {action['combat_card_index']} targetid:")
        elif kind == 'NetUsePotionAction':
            self.owner = ('UsePotionAction ', f" index: {action['potion_index']} target:")
        elif event.get('event_type') == 'HookAction':
            self.owner = ('GenericHookGameAction ', f" id {event['hook_id']} owner ")

    def completed(self, state):
        if self.owner is None or state['player'].get('pending'):
            return
        if self.checks:
            if self.cursor >= len(self.checks):
                raise ValueError('recorded completed-action checkpoint missing')
            checkpoint = self.checks[self.cursor]
            if not all(part in checkpoint['context'] for part in self.owner):
                raise ValueError('recorded checkpoint action identity mismatch')
            self.cursor += 1
            if checkpoint.get('full_state'):
                check_native_snapshot(state, checkpoint['full_state'])
                self.validated += 1
        self.owner = None

    def finish(self):
        if self.cursor != len(self.checks):
            raise ValueError('recorded completed-action checkpoints remain unconsumed')


#: Native ids of the six AfterEnergyReset listeners, mapped to their
#: canonical player slots (`boundary.rs`). Shared by the production review
#: root (`rust_review._replay_opening`) and the eval census (#3020).
AFTER_ENERGY_RESET_NATIVE_IDS = {
    "GENESIS_POWER": "genesis",
    "STAR_NEXT_TURN_POWER": "star_next_turn",
    "ENERGY_NEXT_TURN_POWER": "energy_next_turn",
    "RADIANCE_POWER": "radiance",
    "LIGHTNING_ROD_POWER": "lightning_rod",
    "SPINNER_POWER": "spinner",
}


def native_after_energy_reset_powers(creature):
    """Return ordered `(canonical_name, amount)` checkpoint facts.

    CreatureState.Serialize preserves `Creature.Powers` list order (native
    v0.111.0, 0x3d980c IL009c); NetFullCombatState carries that same list at
    0x119988 IL018e-01d9. This deliberately reads one matching checkpoint
    and never fills gaps from a reconstructed combat state.
    """
    if "powers" not in creature:
        raise ValueError("missing native AfterEnergyReset power list")
    powers = creature["powers"]
    if not isinstance(powers, (list, tuple)):
        raise ValueError("malformed native AfterEnergyReset power list")
    entries = []
    seen = set()
    for power in powers:
        if not isinstance(power, dict):
            raise ValueError("malformed native AfterEnergyReset power list")
        name = power.get("id")
        if name in AFTER_ENERGY_RESET_NATIVE_IDS:
            amount = power.get("amount")
            if (not isinstance(amount, int) or isinstance(amount, bool)
                    or amount <= 0 or name in seen):
                raise ValueError("malformed native AfterEnergyReset power list")
            seen.add(name)
            entries.append((AFTER_ENERGY_RESET_NATIVE_IDS[name], amount))
    return entries
