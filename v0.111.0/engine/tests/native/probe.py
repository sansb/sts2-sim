#!/usr/bin/env python3
"""Native probe driver: scripted plays on the real game, headless.

Runs the scenarios in the `*.spec.json` files beside it on an archived
`sts2.dll` through the headless harness (`sim/<build>/python/harness`, local
only: the DLL is not in the repo), and writes what the game did. The crate's
tests read the recorded results (`tests/native/*.native.json`) and compare
the engine with them.

A scenario is `{character, relics, deck, encounter?, seed?, ascension?,
script}`. Script steps: `["energy", n]`, `["arrange", {hand, draw,
discard}]` (index 0 first; the Draw pile's top), `["hp", combat_id, n]`,
`["power", combat_id, "PowerClass", amount]` (`PowerCmd.Apply`, the creature
as its own applier), `["potion", "POTION_ID"]` (grants it),
`["use", slot, target]`, `["play", label, target]` (a `PlayCardAction`),
`["autoplay", label]` (`CardCmd.AutoPlay` with no target), `["end"]` (an
`EndPlayerTurnAction`, through to the next player turn) and
`["snap", label]`. A card label is its id, a `+` per upgrade, and
`@ENCHANTMENT:amount` in a deck entry.

Every play goes through the game's own `PlayCardAction`. A choice is
answered through the `TestCardSelector` the game ships, pushed as the LOCAL
selector: the action really pauses in `GatheringPlayerChoice`, and only then
is the local selector asked (`CardSelectCmd/<FromCombatPile>d__20` RVA
`0x3e5e84` IL_0077-00ae, IL_0276-0303).

Usage (needs the harness venv and an archived game build):

    export STS2_HARNESS_DIR=<repo>/sim/v0.111.0/python/harness
    $STS2_HARNESS_DIR/venv/bin/python probe.py list
    $STS2_HARNESS_DIR/venv/bin/python probe.py run NAME 0,1
    $STS2_HARNESS_DIR/venv/bin/python probe.py drive --out . [NAME ...]
    STS2_WAITS=real $STS2_HARNESS_DIR/venv/bin/python probe.py drive --out . [NAME ...]

`run NAME ANSWERS` plays one scenario with a comma list of option indices
(one per choice, `-` for none) and prints `NAT <json>`. `drive` enumerates
every answer sequence the game accepts, one CLR process each, and writes
`NAME.native.json`.

`STS2_WAITS=real[:normal|fast|instant][:always]` runs the scenario with
`Cmd.Wait` as a timer from its first `snap` on (or from a `["real_waits"]`
step), the way a game someone is playing runs it, and `drive` then writes
`NAME.native.waits.json` with the mode and the concurrency report in it
(`harness/real_waits.py`, harness README "Real waits", #3931). Unset, nothing
changes: the committed `*.native.json` were recorded that way and still
reproduce.

Each snapshot holds the player's HP, Block, Energy and powers (sorted, and
again as `power_order`, the order `Creature.Powers` holds them in), the orb
queue and its capacity, every creature
in `CombatState.Creatures` order with its CombatId, every pile in order, the
Hand's resolved costs, the nine combat RNG streams (counter and the four
xoshiro words), the CardPlay counts and the history entry counts.
"""
import functools
import json
import os
import sys
import time
from pathlib import Path

print = functools.partial(print, flush=True)
HERE = Path(__file__).resolve().parent
DLL_SHA256 = "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4"

# native RunRngSet stream -> the engine's RngStream wire key
STREAMS = {
    "Shuffle": "rng", "Niche": "niche", "MonsterAi": "ai",
    "CombatCardSelection": "sel", "CombatCardGeneration": "generation",
    "CombatTargets": "targets", "CombatEnergyCosts": "energy_costs",
    "CombatOrbGeneration": "combat_orbs",
    "CombatPotionGeneration": "potion_generation",
}


def scenarios():
    """The scenarios beside this file: every `*.spec.json`, keyed by scenario name."""
    out = {}
    for path in sorted(HERE.glob("*.spec.json")):
        out.update(json.loads(path.read_text()))
    return out


# --------------------------------------------------------------------------
# one scenario, one CLR process
# --------------------------------------------------------------------------
def run(name, answers):
    # host.py expects the .NET runtime beside it; a fresh worktree has none.
    sys.path.insert(0, os.environ.get("STS2_HARNESS_DIR", str(HERE)))
    import experiment as E
    import host
    import real_waits
    import System
    from experiment import _method, _model_by_id, call, find, prop
    from System import Array, Int32, Object, UInt64

    waits = real_waits.from_env()

    ALL = E.ALL
    spec = scenarios()[name]

    def fields(obj):
        out = {}
        t = obj.GetType()
        while t is not None:
            for f in t.GetFields(ALL):
                if not f.IsStatic:
                    out.setdefault(str(f.Name), f)
            t = t.BaseType
        return out

    def rng_state(run_state):
        rng = prop(run_state, "Rng")
        out = {}
        for native, key in STREAMS.items():
            stream = prop(rng, native)
            fs = fields(stream)
            words = None
            for f in fs.values():
                if "MegaRandom" in str(f.FieldType.Name):
                    inner = f.GetValue(stream)
                    words = [(n, int(str(m.GetValue(inner))))
                             for n, m in sorted(fields(inner).items())
                             if str(m.FieldType.Name) == "UInt64"]
            out[key] = {"counter": int(fs["_counter"].GetValue(stream)), "words": words}
        return out

    def card_label(c):
        label = str(prop(c, "Id")).split(".")[-1] + "+" * int(prop(c, "CurrentUpgradeLevel"))
        enchantment = prop(c, "Enchantment")
        if enchantment is not None:
            label += "@" + str(prop(enchantment, "Id")).split(".")[-1]
        if "Ethereal" in [str(k) for k in prop(c, "Keywords")]:
            label += "[Ethereal]"
        return label

    def damage_base(c):
        """The card's `Damage` DynamicVar base as the game holds it (a Decimal
        in text), or None for a card without one. Snapshots list it per pile
        as `damage_bases`, beside `piles`."""
        try:
            return str(prop(call(prop(c, "DynamicVars"), "get_Damage"), "BaseValue"))
        except Exception:
            return None

    def creature_name(c):
        monster = prop(c, "Monster")
        if monster is None:
            return "PLAYER"
        return str(prop(monster, "Id")).split(".")[-1]

    def powers(creature):
        return sorted((str(p.GetType().Name), int(prop(p, "Amount")))
                      for p in prop(creature, "Powers"))

    piles = ("Hand", "DrawPile", "DiscardPile", "ExhaustPile", "PlayPile")

    def snap(f):
        pc = prop(f.player, "Creature")
        mons = [dict(id=int(prop(c, "CombatId")), hp=int(prop(c, "CurrentHp")),
                     max_hp=int(prop(c, "MaxHp")), block=int(prop(c, "Block")),
                     powers=powers(c))
                for c in prop(f.cs, "Creatures") if prop(c, "Monster") is not None]
        creatures = [dict(id=int(prop(c, "CombatId")), name=creature_name(c),
                          hp=int(prop(c, "CurrentHp")), max_hp=int(prop(c, "MaxHp")),
                          block=int(prop(c, "Block")), powers=powers(c),
                          pet=bool(prop(c, "IsPet")), dead=bool(prop(c, "IsDead")))
                     for c in prop(f.cs, "Creatures")]
        started = finished = 0
        history = {}
        for e in prop(prop(f.cm, "History"), "Entries"):
            kind = str(e.GetType().Name)
            started += kind == "CardPlayStartedEntry"
            finished += kind == "CardPlayFinishedEntry"
            history[kind] = history.get(kind, 0) + 1
        return dict(
            hp=int(prop(pc, "CurrentHp")), max_hp=int(prop(pc, "MaxHp")),
            block=int(prop(pc, "Block")), energy=int(prop(f.pcs, "Energy")),
            powers=powers(pc),
            # The player's power list as the game holds it (application order).
            power_order=[str(p.GetType().Name) for p in prop(pc, "Powers")],
            orbs=[str(o.GetType().Name) for o in prop(prop(f.pcs, "OrbQueue"), "Orbs")],
            orb_slots=int(prop(prop(f.pcs, "OrbQueue"), "Capacity")),
            mons=mons, creatures=creatures,
            hand_costs=[int(call(prop(c, "EnergyCost"), "GetResolved"))
                        for c in prop(prop(f.pcs, "Hand"), "Cards")],
            piles={p: [card_label(c) for c in prop(prop(f.pcs, p), "Cards")] for p in piles},
            damage_bases={p: [damage_base(c) for c in prop(prop(f.pcs, p), "Cards")]
                          for p in piles},
            history=history, plays_started=started, plays_finished=finished,
            over=bool(f.over), rng=rng_state(f.run))

    def arrange(f, layout):
        """Each named pile holds exactly the listed cards, index 0 first (the
        Draw pile's top). The scenario's deck is exactly those cards."""
        pool = [c for p in piles for c in prop(prop(f.pcs, p), "Cards")]
        for c in pool:
            pile = prop(c, "Pile")
            remove = _method(pile.GetType(), "RemoveInternal")
            remove.Invoke(pile, Array[Object]([c, True][:remove.GetParameters().Length]))
        for key, native in (("hand", "Hand"), ("draw", "DrawPile"), ("discard", "DiscardPile")):
            pile = prop(f.pcs, native)
            add = _method(pile.GetType(), "AddInternal", 3)
            for idx, label in enumerate(layout.get(key, [])):
                pick = next((c for c in pool if card_label(c) == label), None)
                assert pick is not None, (label, [card_label(c) for c in pool])
                pool.remove(pick)
                add.Invoke(pile, Array[Object]([pick, Int32(idx), True]))
        assert not pool, [card_label(c) for c in pool]

    def pump(f, action, want_pause, timeout_s=10.0):
        t0 = time.time()
        state = None
        while time.time() - t0 < timeout_s:
            state = str(prop(action, "State"))
            if state in ("Finished", "Cancelled"):
                return state
            if want_pause and state == "GatheringPlayerChoice":
                return state
            call(f.ex, "Unpause")
            waits.pump(f.ex)
            # The game runs on this thread only (harness `one_thread.py`): no
            # sleep for another one, a bounded wait for a `Task.Delay` to post.
            waits.idle(0.02)
        waits.stalled_on("step timed out with the action in state " + str(state))
        return "timeout:" + str(state)

    E._boot_and_init()
    waits.install()
    # Headless has no loc tables: `CardModel.SelectionScreenPrompt` (RVA
    # `0x7c304`) throws on `LocString.Exists`, and `CardSelectorPrefs`' ctor
    # adds the count to that prompt's variables. Both are display-only.
    # Stratagem's prompt is `PowerModel.SelectionScreenPrompt`, the same shape.
    for owner in ("CardModel", "PowerModel"):
        host.patch_skip(find(owner), "get_SelectionScreenPrompt", find("LocString"), "uninit")
    host.patch_skip(find("LocString"), "AddObj")
    player = E._make_player(spec["character"])
    deck = prop(player, "Deck")
    _method(deck.GetType(), "Clear", 1).Invoke(deck, Array[Object]([False]))
    add = _method(deck.GetType(), "AddInternal", 3)
    for idx, label in enumerate(spec["deck"]):
        card, _, enchantment = label.partition("@")
        clone = call(_model_by_id("CARD." + card.rstrip("+"), "CardModel"), "MutableClone")
        for _ in range(card.count("+")):
            call(clone, "UpgradeInternal")
        if enchantment:
            ench_id, _, amount = enchantment.partition(":")
            model = call(_model_by_id("ENCHANTMENT." + ench_id, "EnchantmentModel"), "MutableClone")
            _method(find("CardModel"), "EnchantInternal").Invoke(
                clone, Array[Object]([model, System.Decimal(int(amount or 1))]))
        add.Invoke(deck, Array[Object]([clone, Int32(idx), False]))
    E._grant_relics(player, ["RELIC." + r for r in spec["relics"]])
    E._grant_potions(player, ["POTION." + p for p in spec.get("potions", [])])
    f = E._Fight(player, spec["character"], int(spec.get("ascension", 0)), [spec.get("act", "Glory")],
                 "ENCOUNTER." + spec.get("encounter", "TOADPOLES_WEAK"), spec.get("seed", "NATIVE"))

    def choice_context():
        hpc = find("HookPlayerChoiceContext")
        ga_type = find("GameActionType")
        ga_none = System.Enum.Parse(ga_type, list(System.Enum.GetNames(ga_type))[0])
        ctor = [c for c in hpc.GetConstructors(ALL)
                if str(c.GetParameters()[0].ParameterType.Name) == "Player"][0]
        return ctor.Invoke(Array[Object]([player, UInt64(0), ga_none]))

    def wait(task):
        waits.wait(f.ex, task)
    selector = [c for c in find("TestCardSelector").GetConstructors(ALL)
                if not c.IsStatic][0].Invoke(Array[Object]([]))
    _method(find("CardSelectCmd"), "PushSelector").Invoke(None, Array[Object]([selector, True]))
    out = []
    pending = iter(answers)
    # Real waits start at the scenario's root: its setup ran headless.
    begin_at = next((i for i, s in enumerate(spec["script"]) if s[0] in ("real_waits", "snap")), 0)
    for index, step in enumerate(spec["script"]):
        kind = step[0]
        if index == begin_at:
            waits.begin(f)
        if kind == "real_waits":
            pass
        elif kind == "arrange":
            arrange(f, step[1])
        elif kind == "energy":
            _method(f.pcs.GetType(), "set_Energy").Invoke(f.pcs, Array[Object]([Int32(step[1])]))
        elif kind == "hp":
            for c in prop(f.cs, "Creatures"):
                if int(prop(c, "CombatId")) == step[1]:
                    call(c, "SetCurrentHpInternal", System.Decimal(step[2]))
        elif kind == "snap":
            out.append(dict(label=step[1], snap=snap(f)))
        elif kind == "power":
            target = f._creature_by_id(step[1])
            apply = [m for m in find("PowerCmd").GetMethods(ALL)
                     if str(m.Name) == "Apply" and m.IsGenericMethod and m.GetParameters().Length == 6
                     and str(m.GetParameters()[1].ParameterType.Name) == "Creature"][0]
            wait(apply.MakeGenericMethod(find(step[2])).Invoke(None, Array[Object](
                [choice_context(), target, System.Decimal(step[3]), target, None, False])))
        elif kind == "autoplay":
            card = next(c for c in prop(prop(f.pcs, "Hand"), "Cards") if card_label(c) == step[1])
            auto = [m for m in find("CardCmd").GetMethods(ALL) if str(m.Name) == "AutoPlay"][0]
            args = [choice_context(), card, None]
            for param in list(auto.GetParameters())[3:]:
                kind_t = param.ParameterType
                args.append(System.Enum.Parse(kind_t, list(System.Enum.GetNames(kind_t))[0])
                            if kind_t.IsEnum else False)
            wait(auto.Invoke(None, Array[Object](args)))
            f._drain()
            out.append(dict(label=f"{index}:autoplay:{step[1]}", snap=snap(f)))
        elif kind == "use":
            state = f.use_potion(step[1], step[2] if len(step) > 2 else None)["state"]
            out.append(dict(label=f"{index}:use:{step[1]}", state=state, snap=snap(f)))
        elif kind == "end":
            state = f.end_turn()["state"]
            out.append(dict(label=f"{index}:end", state=state, snap=snap(f)))
        elif kind == "play":
            label = f"{index}:{step[1]}"
            card = next(c for c in prop(prop(f.pcs, "Hand"), "Cards") if card_label(c) == step[1])
            # More pending answers than any one play of these scenarios asks.
            sources = [call(selector, "SetupForAsyncIndexSelection") for _ in range(4)]
            ctor = [c for c in find("PlayCardAction").GetConstructors(ALL)
                    if not c.IsStatic and c.GetParameters().Length == 2][0]
            action = ctor.Invoke(Array[Object]([card, f._creature_by_id(step[2] if len(step) > 2 else None)]))
            call(f.aqs, "EnqueueAction", action, UInt64(int(str(prop(f.player, "NetId")))))
            for park, source in enumerate(sources):
                state = pump(f, action, True)
                if state != "GatheringPlayerChoice":
                    break
                # Whatever the game runs while the choice is open has run.
                waits.drain(f.ex)
                out.append(dict(label=f"{label}:parked-{park}", state=state, snap=snap(f)))
                answer = next(pending, None)
                if answer is None:
                    # More choices than answers: say so and stop.
                    out.append(dict(label="unanswered"))
                    print("NAT " + json.dumps(out))
                    os._exit(0)
                picked = E.generic_list(System.Int32)
                picked.Add(Int32(int(answer)))
                call(source, "SetResult", picked)
            state = pump(f, action, False)
            waits.drain(f.ex)
            call(selector, "Cleanup")
            out.append(dict(label=f"{label}:done", state=state, snap=snap(f)))
        else:
            raise SystemExit(f"bad step {step}")
    if waits.active:
        waits.drain(f.ex)
        out.append(dict(label="waits", report=waits.report()))
    print("NAT " + json.dumps(out))


# --------------------------------------------------------------------------
# every answer sequence of a scenario
# --------------------------------------------------------------------------
def _drive():
    """`probe_drive`, the harness module both native probes share for the parent half of `drive`."""
    sys.path.insert(0, os.environ.get("STS2_HARNESS_DIR", str(HERE)))
    import probe_drive
    return probe_drive


def _run_child(name, answers):
    # None only for an answer past the last option. A child that stalled,
    # threw anywhere else or printed no trace raises: never "no such option".
    return _drive().run_child(Path(__file__).resolve(), name, answers)


def drive(name):
    runs = _drive().explore(lambda answers: _run_child(name, answers))
    out = dict(build="v0.111.0", dll_sha256=DLL_SHA256, name=name,
               spec=scenarios()[name], runs=runs)
    if os.environ.get("STS2_WAITS", "off") not in ("", "off"):
        out["waits"] = os.environ["STS2_WAITS"]
    return out


def main(argv):
    if len(argv) >= 2 and argv[0] == "run":
        answers = [a for a in argv[2].split(",") if a != "-"] if len(argv) > 2 else []
        run(argv[1], answers)
    elif argv and argv[0] == "drive":
        out = Path(argv[argv.index("--out") + 1])
        names = [a for i, a in enumerate(argv[1:], 1)
                 if a != "--out" and argv[i - 1] != "--out"] or sorted(scenarios())
        out.mkdir(parents=True, exist_ok=True)
        for name in names:
            trace = drive(name)
            text = json.dumps(trace, separators=(",", ":"), sort_keys=True)
            suffix = "native.waits" if "waits" in trace else "native"
            (out / f"{name}.{suffix}.json").write_text(text + "\n")
            print(name, [run["answers"] for run in trace["runs"]])
    elif argv and argv[0] == "list":
        print("\n".join(sorted(scenarios())))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
