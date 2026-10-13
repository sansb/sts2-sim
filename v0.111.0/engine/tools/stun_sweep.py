#!/usr/bin/env python3
"""The stun sweep: no admitted fight may refuse or get stuck once a monster is stunned (#3823).

For every encounter of the roster fixtures, opened through the entry-facts
opening with a Whistle in the deck, the sweep stuns monsters and plays seeded
random legal lines, and after EVERY step projects the state, loads it into a
second session and requires that session to admit it and project the same
document. It reports three things, and exits 1 on any of them:

* a refusal of a legal action or of a projection (the fight refused mid-line);
* a state that does not reload, or reloads to a different document;
* a fight that is not over and has no legal action (stuck).

A monster is stunned two ways. `whistle`: the public play of a Whistle on it.
`stun`: the document state that play leaves (`override: STUNNED` over the
parked telegraph), written into the projected document and loaded. The second
is there because Whistle also deals 33 damage, which kills the small monsters
(a Two-Tailed Rat, a slime) before their stunned turn can be seen; it is used
only where the engine admits the edited document, so it can add a state but
never excuse one. Lines:

* scheduled: for each monster and each of the first `--turns` turns, stun it
  on that turn (each way), then play random legal actions;
* random: every turn each living monster is stunned with probability 0.3, and
  the plays are random (the decks hold Whistles and Strikes, so stunned
  monsters are hit and killed, and stunned again on consecutive turns).

An encounter whose opening the engine refuses with a Whistle in the deck is
listed with the refusal's name and is not a finding: that is admission saying
so up front, which is the stopgap #3823 is removing kind by kind.

    stun_sweep.py                              # every encounter
    stun_sweep.py --match KNIGHTS --lines 4    # a slice, more lines
    stun_sweep.py --out target/stun_sweep.json

It is a measurement tool, like `composition_sweep.py`: run it on the release
binary after a change to a monster's move machine or to the stun, and read the
last line. "ok" means nothing was met in these lines, never "certified"; the
certification of a stunned turn is `engine::native_stun_traces`, against the
game's own recorded turns.
"""
import argparse
import glob
import json
import pathlib
import random
import subprocess
import sys
import tempfile

CRATE = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CRATE.parent / "python"))
sys.path.insert(0, str(CRATE / "tools"))
import sts2_rng  # noqa: E402
from diff_serve_client import DiffServe  # noqa: E402

BUILD = "v0.111.0"
STREAMS = ("shuffle", "niche", "monster_ai", "combat_card_selection", "combat_card_generation",
           "combat_targets", "combat_energy_costs", "combat_orbs", "combat_potion_generation")
DECKS = {
    "whistles": ["CARD.WHISTLE"] * 12,
    "mixed": ["CARD.WHISTLE"] * 5 + ["CARD.STRIKE_IRONCLAD"] * 5 + ["CARD.DEFEND_IRONCLAD"] * 2 + ["CARD.BASH"],
    "strikes": ["CARD.WHISTLE"] * 6 + ["CARD.STRIKE_IRONCLAD"] * 6,
}
MAX_TURNS = 14


def encounters():
    found = set()
    for path in glob.glob(str(CRATE / "fixtures" / "*_rosters_v1.json")):
        found.update(case["encounter"] for case in json.loads(pathlib.Path(path).read_text())["cases"])
    return sorted(found)


def streams(seed):
    pascal = {name: "".join(part.capitalize() for part in name.split("_")) for name in STREAMS}
    rng_set = sts2_rng.RunRngSet(seed, {key: 0 for key in pascal.values()}, build=BUILD)
    out = {}
    for name, key in pascal.items():
        if key not in rng_set.rngs:
            key = {"CombatOrbs": "CombatOrbGeneration"}.get(key, key)
        words = rng_set.rngs[key]._random
        out[name] = {"counter": 0, "s0": words.s0, "s1": words.s1, "s2": words.s2, "s3": words.s3}
    return out


def facts(encounter, seed, deck, node_index, hp):
    node_type = "boss" if encounter.endswith("_BOSS") else "elite" if encounter.endswith("_ELITE") else "monster"
    table = streams(seed)
    return {
        "schema": "sts-sim-entry-v1", "game_build": BUILD, "save_schema_version": 20,
        "seed": seed, "character": "CHARACTER.IRONCLAD",
        "entry": {
            "encounter_id": encounter, "node_index": node_index, "node_type": node_type,
            "deck_entering": [{"id": card, "upgrade_level": 0} for card in deck],
            "relics_entering": ["RELIC.BURNING_BLOOD"], "potions_entering": [],
            "max_potion_slot_count": None, "potion_slots_entering": [],
            "hp_entering": hp, "max_hp_entering": hp, "gold_entering": 99, "ascension": 10,
            "relic_counters": None, "tea_set_charged": None, "fake_tea_set_charged": None,
            "fur_coat_active": None, "relics_entering_dispatch_ordered": True, "entry_ambiguous": False,
        },
        "counters": {name: 0 for name in table}, "streams": table,
        "unlocks": {"unlocked_card_pool_epochs": None, "fully_unlocked_card_pool": True,
                    "fully_unlocked_potion_pool": None},
    }


def opening(binary, document):
    with tempfile.TemporaryDirectory(prefix="stun-sweep-") as scratch:
        path = pathlib.Path(scratch) / "facts.json"
        path.write_text(json.dumps(document))
        done = subprocess.run([str(binary), "entry", "--build", BUILD, "--facts", str(path), "--opening"],
                              text=True, capture_output=True, timeout=120)
    if done.returncode:
        raise RuntimeError("entry --opening failed: " + done.stderr[-400:])
    return json.loads(done.stdout)


class Line:
    """One fight line: a driven session and a second one every state is reloaded into."""

    def __init__(self, binary, root):
        self.main, self.mirror = DiffServe(binary), DiffServe(binary)
        self.trace, self.counts = [], {"steps": 0, "whistles": 0, "stuns": 0, "stunned_states": 0, "kills_of_stunned": 0}
        self.finding = None
        loaded = self.main.load(root)
        if "ok" not in loaded:
            self.finding = "root not loaded: " + json.dumps(loaded)[:300]

    def close(self):
        self.main.close()
        self.mirror.close()

    def fail(self, what):
        self.finding = f"{what} | after {' '.join(self.trace[-14:])}"
        return None

    def state(self):
        """The projected state, reloaded and compared; None (and a finding) when it fails."""
        projected = self.main.project()
        if "ok" not in projected:
            return self.fail("PROJECT refused: " + json.dumps(projected)[:300])
        state = projected["ok"].get("state", projected["ok"])
        reloaded = self.mirror.load(state)
        if "ok" not in reloaded:
            shape = [(m["kind"], m["hp"], m.get("override"), m.get("next_move", m.get("loop_pos", 0))) for m in state["monsters"]]
            return self.fail(f"RELOAD refused: {json.dumps(reloaded)[:300]} monsters={shape}")
        again = self.mirror.project()
        if "ok" not in again or again["ok"].get("state", again["ok"]) != state:
            return self.fail("RELOAD projects a different document")
        self.counts["stunned_states"] += any(m.get("override") == "STUNNED" for m in state["monsters"])
        return state

    def stun(self, state, index):
        """Write the stunned state on monster `index` and load it.

        Returns the edited document, or None when the monster cannot take it
        or the engine does not admit the edit."""
        if index >= len(state["monsters"]):
            return None
        monster = state["monsters"][index]
        if monster["hp"] <= 0 or monster.get("override") or monster.get("forced_follow_up"):
            return None
        intents = self.main.request({"cmd": "intents"})
        shown = None
        if "ok" in intents:
            shown = next((row.get("intent") for row in intents["ok"]["intents"] if row["uid"] == monster.get("uid", 0)), None)
        # The follow-up names the parked row: its own name for the loops whose
        # rows are all named, the generic token for every other machine.
        for follow_up in ("PARKED_TELEGRAPH", shown):
            if not follow_up:
                continue
            edited = json.loads(json.dumps(state))
            edited["monsters"][index]["override"] = "STUNNED"
            edited["monsters"][index]["forced_follow_up"] = follow_up
            probe = self.mirror.load(edited)
            if "ok" in probe:
                self.main.load(edited)
                self.trace.append(f"stun>{index}")
                self.counts["stuns"] += 1
                return edited
        return None

    def run(self, rng, schedule, random_stuns):
        """`schedule`: (turn, monster index, 'whistle' | 'stun') or None."""
        for turn in range(1, MAX_TURNS + 1):
            scheduled = schedule is not None and schedule[0] == turn
            first = True
            while True:
                state = self.state()
                if state is None:
                    return
                if first:
                    first = False
                    if scheduled and schedule[2] == "stun" and self.stun(state, schedule[1]):
                        continue
                    if random_stuns:
                        hit = False
                        for index in range(len(state["monsters"])):
                            edited = self.stun(state, index) if rng.random() < 0.3 else None
                            if edited is not None:
                                state, hit = edited, True
                        if hit:
                            continue
                legal = self.main.legal()
                if "ok" not in legal:
                    return self.fail("LEGAL refused: " + json.dumps(legal)[:300])
                actions = legal["ok"]["actions"]
                if not actions:
                    return self.fail("STUCK: no legal action in a fight that is not over")
                hand = {card["uid"]: card["id"] for card in state["piles"]["hand"]}
                action = None
                if scheduled and schedule[2] == "whistle":
                    scheduled = False
                    action = next((a for a in actions if a.get("kind") == "play" and hand.get(a.get("uid")) == "WHISTLE"
                                   and a.get("target") == schedule[1]), None)
                if action is None:
                    plays = [a for a in actions if a.get("kind") != "end"]
                    ends = [a for a in actions if a.get("kind") == "end"]
                    action = rng.choice(plays) if plays and (not ends or rng.random() < 0.75) else (ends[0] if ends else rng.choice(actions))
                name = hand.get(action.get("uid"), action.get("kind"))
                self.trace.append(f"t{turn}:{name[:4]}>{action.get('target', '')}")
                target = action.get("target")
                stunned_target = (isinstance(target, int) and target < len(state["monsters"])
                                  and state["monsters"][target].get("override") == "STUNNED")
                applied = self.main.apply(action)
                self.counts["steps"] += 1
                if "ok" not in applied:
                    shape = [(m["kind"], m["hp"], m.get("override"), m.get("next_move", m.get("loop_pos", 0))) for m in state["monsters"]]
                    return self.fail(f"APPLY {name} target={target} refused: {json.dumps(applied.get('refusal') or applied)[:300]} monsters={shape}")
                self.counts["whistles"] += name == "WHISTLE"
                if applied["ok"].get("over") or applied["ok"].get("terminal"):
                    return
                if stunned_target:
                    after = self.main.project()
                    if "ok" in after:
                        monsters = after["ok"].get("state", after["ok"])["monsters"]
                        uid = state["monsters"][target].get("uid", 0)
                        self.counts["kills_of_stunned"] += not any(m.get("uid", 0) == uid and m["hp"] > 0 for m in monsters)
                if action.get("kind") == "end":
                    break


def sweep_encounter(binary, encounter, args):
    totals = {"lines": 0, "steps": 0, "whistles": 0, "stuns": 0, "stunned_states": 0, "kills_of_stunned": 0}
    findings, status, rosters = [], "ok", set()
    for seed in args.seeds:
        for deck_name, deck in DECKS.items():
            for node_index in range(4):
                try:
                    root = opening(binary, facts(encounter, seed, deck, node_index, args.hp))
                except RuntimeError as error:
                    return {"encounter": encounter, "status": "opening error: " + str(error)[:120], **totals, "findings": []}
                if root.get("schema") != "sts-sim-canonical-v2":
                    refusal = root.get("opening") or root.get("refusal") or root
                    return {"encounter": encounter, "status": "opening refused: " + json.dumps(refusal)[:150], **totals, "findings": []}
                roster = (seed, deck_name, tuple(m["kind"] for m in root["monsters"]))
                if node_index and roster in rosters:
                    continue
                rosters.add(roster)
                probe = DiffServe(binary)
                admitted = probe.load(root)
                probe.close()
                if "ok" not in admitted:
                    return {"encounter": encounter, "status": "not admitted: " + json.dumps(admitted.get("missing") or admitted)[:150],
                            **totals, "findings": []}
                count = len(root["monsters"])
                # two indexes past the opening roster reach a summoned monster
                schedules = [(None, False), (None, True), (None, True)]
                schedules += [((turn, index, how), False) for turn in range(1, args.turns + 1)
                              for index in range(count + 2) for how in ("whistle", "stun")]
                for schedule, random_stuns in schedules:
                    for repeat in range(args.lines):
                        rng = random.Random(f"{encounter}|{seed}|{deck_name}|{node_index}|{schedule}|{random_stuns}|{repeat}")
                        line = Line(binary, root)
                        try:
                            if line.finding is None:
                                line.run(rng, schedule, random_stuns)
                        finally:
                            line.close()
                        totals["lines"] += 1
                        for key, value in line.counts.items():
                            totals[key] += value
                        if line.finding:
                            findings.append(f"{seed} {deck_name} node {node_index} {schedule} random_stuns={random_stuns} #{repeat}: {line.finding}")
    return {"encounter": encounter, "status": status, **totals, "findings": findings}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--binary", type=pathlib.Path, default=CRATE / "target" / "release" / "sts-sim")
    parser.add_argument("--match", action="append", default=[], help="only encounters whose id contains this (repeatable)")
    parser.add_argument("--lines", type=int, default=2, help="random lines per schedule (default 2)")
    parser.add_argument("--turns", type=int, default=4, help="scheduled stuns on turns 1..N (default 4)")
    parser.add_argument("--seeds", type=lambda text: text.split(","), default=["STUNPROBE", "STUNPROBE2"])
    parser.add_argument("--hp", type=int, default=9000, help="the player's HP, high so the machines run long (default 9000)")
    parser.add_argument("--out", type=pathlib.Path, help="write every encounter's row as JSON")
    args = parser.parse_args()
    if not args.binary.exists():
        sys.exit(f"no release binary at {args.binary}: cargo build --release --locked")
    rows = []
    for encounter in encounters():
        if args.match and not any(text in encounter for text in args.match):
            continue
        row = sweep_encounter(args.binary, encounter, args)
        rows.append(row)
        print(f"{encounter:48s} {row['status'][:90]:16s} lines={row['lines']} whistles={row['whistles']} stuns={row['stuns']} "
              f"stunned_states={row['stunned_states']} kills_of_stunned={row['kills_of_stunned']} steps={row['steps']} "
              f"findings={len(row['findings'])}", flush=True)
        seen = {}
        for finding in row["findings"]:
            seen.setdefault(finding.split(": ", 1)[1][:80], finding)
        for finding in list(seen.values())[:6]:
            print("     ", finding[:900], flush=True)
    admitted = [row for row in rows if row["status"] == "ok"]
    total = {key: sum(row[key] for row in rows) for key in ("lines", "whistles", "stuns", "stunned_states", "kills_of_stunned", "steps")}
    findings = sum(len(row["findings"]) for row in rows)
    print(f"encounters {len(rows)}: {len(admitted)} admit a Whistle, {len(rows) - len(admitted)} refuse at the opening or admission; "
          + " ".join(f"{key}={value}" for key, value in total.items()) + f" findings={findings}")
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps({"rows": rows, "totals": total, "findings": findings}, indent=1))
    sys.exit(1 if findings else 0)


if __name__ == "__main__":
    main()
