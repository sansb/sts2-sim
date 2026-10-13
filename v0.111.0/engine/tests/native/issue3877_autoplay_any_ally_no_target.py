"""Native witness for #3877: an AnyAlly card in a fight with no other player.

Drives the archived v0.111.0 `sts2.dll` through the headless harness
(`sim/v0.111.0/python/harness`, local only: the DLL is not in the repo).
One case per process (the CLR boots once):

    HARNESS=sim/v0.111.0/python/harness
    $HARNESS/venv/bin/python issue3877_autoplay_any_ally_no_target.py CASE

CASE is `<mode>:<CARD_ID>[:osty]`. An Ironclad (a Necrobinder with `:osty`,
who has a living Osty) faces `ENCOUNTER.CULTISTS_NORMAL` with a deck of seven
copies of the card.

* `manualnone` / `manualself` / `manualpet`: the card's own `CanPlay`, then a
  `PlayCardAction` with no target, aimed at the owner, or aimed at Osty.
* `auto`: `CardCmd.AutoPlay(ctx, card, null, AutoPlayType.Default, false,
  false)` on a Hand card, with every RNG counter read before and after.
* `mayhem`: Mayhem 1 is applied and the turn ended, so the game's own
  `MayhemPower` turns the card up at the next turn start.

The output is one `RESULT` JSON line; the committed
`issue3877_autoplay_any_ally_no_target.json` is those lines keyed by case.
"""
import json
import sys

H = '/Users/seanbloomfield/Documents/StsHistoryViewer/sim/v0.111.0/python/harness'
sys.path.insert(0, H)
import experiment as X  # noqa: E402
from experiment import prop, call, find, ALL  # noqa: E402
import System  # noqa: E402
from System import Array, Object, UInt64, Enum  # noqa: E402

case = sys.argv[1]
bits = case.split(':')
mode, card_id = bits[0], bits[1]
osty = len(bits) > 2 and bits[2] == 'osty'
character = 'Necrobinder' if osty else 'Ironclad'
X._boot_and_init()
player = X._make_player(character)
X._override_deck(player, ['CARD.' + card_id] * 7)
fight = X._Fight(player, character, 0, ['Glory'], 'ENCOUNTER.CULTISTS_NORMAL', 'TESTSEED')
me = prop(player, 'Creature')


def ctx():
    HPC = find('HookPlayerChoiceContext')
    GAType = find('GameActionType')
    ga_none = Enum.Parse(GAType, list(Enum.GetNames(GAType))[0])
    ctor = [c for c in HPC.GetConstructors(ALL) if str(c.GetParameters()[0].ParameterType.Name) == 'Player'][0]
    return ctor.Invoke(Array[Object]([player, UInt64(0), ga_none]))


def apply(power, amount, target):
    cands = [m for m in find('PowerCmd').GetMethods(ALL) if str(m.Name) == 'Apply' and m.IsGenericMethod
             and m.GetParameters().Length == 6 and str(m.GetParameters()[1].ParameterType.Name) == 'Creature']
    t = cands[0].MakeGenericMethod(find(power)).Invoke(
        None, Array[Object]([ctx(), target, System.Decimal(amount), target, None, False]))
    call(call(t, 'GetAwaiter'), 'GetResult')


def pile(name):
    return [str(prop(c, 'Id')) for c in prop(prop(fight.pcs, name), 'Cards')]


def piles():
    return {n: len(pile(n)) for n in ('Hand', 'DrawPile', 'DiscardPile', 'ExhaustPile')}


def powers(creature):
    return sorted((str(p.GetType().Name), int(prop(p, 'Amount'))) for p in prop(creature, 'Powers'))


def snapshot():
    return {'piles': piles(), 'energy': int(prop(fight.pcs, 'Energy')), 'block': int(prop(me, 'Block')),
            'hp': int(prop(me, 'CurrentHp')), 'powers': powers(me),
            'creatures': [(int(prop(c, 'CombatId')), str(prop(c, 'Monster') or prop(c, 'Player') and 'PLAYER'),
                           int(prop(c, 'CurrentHp')), int(prop(c, 'Block')))
                          for c in prop(fight.cs, 'Creatures')]}


def hist_types(since):
    types = {}
    for e in fight.history()[since:]:
        types[e['type']] = types.get(e['type'], 0) + 1
    return types


card = fight._hand()[0]
out = {'case': case, 'build': X.build_version(), 'card': str(prop(card, 'Id')),
       'target_type': str(prop(card, 'TargetType')), 'card_type': str(prop(card, 'Type')),
       'before': snapshot()}
allies = [c for c in prop(fight.cs, 'Creatures') if prop(c, 'Monster') is None]
out['player_side'] = [(int(prop(c, 'CombatId')), bool(prop(c, 'IsPlayer')), bool(prop(c, 'IsPet')))
                      for c in allies]
if mode.startswith('manual'):
    can = [m for m in card.GetType().GetMethods(ALL) if str(m.Name) == 'CanPlay' and m.GetParameters().Length == 0]
    out['can_play'] = bool(can[0].Invoke(card, Array[Object]([]))) if can else None
    can2 = [m for m in card.GetType().GetMethods(ALL) if str(m.Name) == 'CanPlay' and m.GetParameters().Length == 2]
    if can2:
        args = Array[Object]([None, None])
        out['can_play_with_reason'] = bool(can2[0].Invoke(card, args))
        out['unplayable_reason'] = str(args[0])
        out['unplayable_preventer'] = str(args[1]) if args[1] is not None else None
    vt = [m for m in card.GetType().GetMethods(ALL) if str(m.Name) == 'IsValidTarget']
    if vt:
        out['is_valid_target'] = {str(int(prop(c, 'CombatId'))): bool(vt[0].Invoke(card, Array[Object]([c])))
                                  for c in prop(fight.cs, 'Creatures')}
    target = {'manualnone': None, 'manualself': 0}.get(mode)
    if mode == 'manualpet':
        target = [int(prop(c, 'CombatId')) for c in allies if bool(prop(c, 'IsPet'))][0]
    out['target'] = target
    if out['can_play']:
        n0 = len(fight.history())
        try:
            out['state'] = fight.play(0, target)['state']
        except Exception as exc:  # noqa: BLE001
            out['state'] = 'raised ' + type(exc).__name__ + ': ' + str(exc)[:160]
        out['history_types'] = hist_types(n0)
        out['after'] = snapshot()
    else:
        # The game offers no play of a card whose CanPlay is false; a forced
        # PlayCardAction crashes the headless process in UI code.
        out['state'] = 'not attempted: CanPlay is false'
elif mode == 'auto':
    rng0 = X._rng_counters(fight.run)
    n0 = len(fight.history())
    ap = [m for m in find('CardCmd').GetMethods(ALL) if str(m.Name) == 'AutoPlay'][0]
    params = list(ap.GetParameters())
    out['autoplay_params'] = [str(p.ParameterType.Name) + ' ' + str(p.Name) for p in params]
    args = [ctx(), card, None]
    for p in params[3:]:
        t = p.ParameterType
        args.append(Enum.Parse(t, list(Enum.GetNames(t))[0]) if t.IsEnum else False)
    t = ap.Invoke(None, Array[Object](args))
    call(call(t, 'GetAwaiter'), 'GetResult')
    fight._drain()
    rng1 = X._rng_counters(fight.run)
    out['rng_delta'] = {k: rng1[k] - rng0[k] for k in rng0 if rng1[k] != rng0[k]}
    out['history_types'] = hist_types(n0)
    out['card_pile_after'] = str(prop(prop(card, 'Pile'), 'Type')) if prop(card, 'Pile') is not None else None
    out['after'] = snapshot()
elif mode == 'mayhem':
    apply('MayhemPower', 1, me)
    rng0 = X._rng_counters(fight.run)
    n0 = len(fight.history())
    out['end_turn'] = fight.end_turn()
    new = fight.history()[n0:]
    out['history_round2_player'] = [(e['type'], e['text'][:60]) for e in new
                                    if e['round'] == 2 and e['side'] == 'Player'
                                    and e['type'] != 'CardDrawnEntry']
    rng1 = X._rng_counters(fight.run)
    out['rng_delta'] = {k: rng1[k] - rng0[k] for k in rng0 if rng1[k] != rng0[k]}
    out['after'] = snapshot()
print('RESULT ' + json.dumps(out, sort_keys=True))
