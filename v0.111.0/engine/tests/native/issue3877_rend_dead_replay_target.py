"""Native witness for #3877: a replayed Rend whose target died in the first body.

Drives the archived v0.111.0 `sts2.dll` through the headless harness
(`sim/v0.111.0/python/harness`, local only: the DLL is not in the repo).
One case per process (the CLR boots once):

    HARNESS=sim/v0.111.0/python/harness
    $HARNESS/venv/bin/python issue3877_rend_dead_replay_target.py CASE

Each case is a Silent against `ENCOUNTER.TOADPOLES_WEAK` with one Rend in an
otherwise starter-card deck. The first Toadpole's hp is set, Thorns 2 is
applied to it, and Rend is played on it once. `glam_*` carries Glam 1 on the
Rend; `echo_*` applies Echo Form 1 to the player first; `punch_*` applies
One-Two Punch 1. The output is one `RESULT` JSON line; the committed
`issue3877_rend_dead_replay_target.json` is those lines keyed by case.
"""
import json
import sys

H = '/Users/seanbloomfield/Documents/StsHistoryViewer/sim/v0.111.0/python/harness'
sys.path.insert(0, H)
import experiment as X  # noqa: E402
from experiment import prop, call, find, _method, ALL  # noqa: E402
import System  # noqa: E402
from System import Array, Object, Int32, UInt64, Enum  # noqa: E402

CASES = {
    # replay source, first Toadpole hp
    'glam_lethal': ('glam', 8),
    'glam_survives': ('glam', 25),
    'echo_lethal': ('EchoFormPower', 8),
    'echo_survives': ('EchoFormPower', 25),
    'punch_lethal': ('OneTwoPunchPower', 8),
    'punch_survives': ('OneTwoPunchPower', 25),
    'plain_lethal': (None, 8),
}
case = sys.argv[1]
source, hp = CASES[case]
X._boot_and_init()
player = X._make_player('Silent')
X._override_deck(player, ['CARD.REND', 'CARD.DEFEND_SILENT', 'CARD.DEFEND_SILENT', 'CARD.DEFEND_SILENT'])
if source == 'glam':
    for c in prop(prop(player, 'Deck'), 'Cards'):
        if 'REND' in str(c):
            em = call(X._model_by_id('ENCHANTMENT.GLAM', 'EnchantmentModel'), 'MutableClone')
            _method(find('CardModel'), 'EnchantInternal').Invoke(c, Array[Object]([em, System.Decimal(1)]))
fight = X._Fight(player, 'Silent', 0, ['Glory'], 'ENCOUNTER.TOADPOLES_WEAK', 'TESTSEED')
me = prop(player, 'Creature')


def apply(power, amount, target):
    HPC = find('HookPlayerChoiceContext')
    GAType = find('GameActionType')
    ga_none = Enum.Parse(GAType, list(Enum.GetNames(GAType))[0])
    ctor = [c for c in HPC.GetConstructors(ALL) if str(c.GetParameters()[0].ParameterType.Name) == 'Player'][0]
    ctx = ctor.Invoke(Array[Object]([player, UInt64(0), ga_none]))
    cands = [m for m in find('PowerCmd').GetMethods(ALL) if str(m.Name) == 'Apply' and m.IsGenericMethod
             and m.GetParameters().Length == 6 and str(m.GetParameters()[1].ParameterType.Name) == 'Creature']
    t = cands[0].MakeGenericMethod(find(power)).Invoke(
        None, Array[Object]([ctx, target, System.Decimal(amount), target, None, False]))
    call(call(t, 'GetAwaiter'), 'GetResult')


def set_hp(creature, val):
    ms = [m for m in creature.GetType().GetMethods(ALL) if str(m.Name) == 'SetCurrentHpInternal']
    pt = ms[0].GetParameters()[0].ParameterType
    ms[0].Invoke(creature, Array[Object]([System.Convert.ChangeType(Int32(val), pt)]))


def monsters():
    return [dict(id=int(prop(c, 'CombatId')), hp=int(prop(c, 'CurrentHp')), dead=bool(prop(c, 'IsDead')))
            for c in prop(fight.cs, 'Creatures') if prop(c, 'Monster') is not None]


def pile(name):
    return [str(prop(c, 'Id')) for c in prop(prop(fight.pcs, name), 'Cards')]


first = [c for c in prop(fight.cs, 'Creatures') if prop(c, 'Monster') is not None][0]
set_hp(first, hp)
apply('ThornsPower', 2, first)
if source not in (None, 'glam'):
    apply(source, 1, me)
out = {'case': case, 'build': X.build_version(), 'monsters_before': monsters(),
       'player_hp_before': int(prop(me, 'CurrentHp'))}
hand = fight._hand()
idx = [i for i, c in enumerate(hand) if 'REND' in str(c)][0]
n0 = len(fight.history())
out['play'] = fight.play(idx, int(prop(first, 'CombatId')))['state']
hist = fight.history()[n0:]
types = {}
for e in hist:
    types[e['type']] = types.get(e['type'], 0) + 1
out['history_types'] = types
out['monsters_after'] = monsters()
out['player_hp_after'] = int(prop(me, 'CurrentHp'))
out['over'] = bool(fight.over)
out['discard'] = pile('DiscardPile')
out['hand'] = pile('Hand')
out['exhaust'] = pile('ExhaustPile')
print('RESULT ' + json.dumps(out, sort_keys=True))
